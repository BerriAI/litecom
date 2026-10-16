use crate::{
    auth::{AgentToken, DeviceCode},
    client::Client,
    config::{self, ClientConfig},
    runner::{self, NativeAgent, RunnerConfig},
    service,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(clap::Args)]
pub struct Setup {
    #[arg(long, env = "LITECOM_RELAY")]
    pub relay: Option<String>,
    /// One-use code from Connect local agent in your signed-in workspace.
    #[arg(long, requires = "relay", conflicts_with = "requester_only")]
    pub connect_code: Option<String>,
    #[arg(long, default_value = "This computer")]
    pub name: String,
    /// Replace an expired or revoked connection after signing in again.
    #[arg(long)]
    pub reconnect: bool,
    /// Choose your main agent without showing the picker.
    #[arg(long, value_parser = ["codex", "claude", "opencode", "cursor", "pi", "openclaw", "hermes", "codex-acp", "claude-agent-acp"], conflicts_with = "acp")]
    pub agent: Option<String>,
    /// Connect another installed ACP agent by executable path or command name.
    #[arg(long)]
    pub acp: Option<String>,
    /// Argument for the ACP executable; repeat for multiple arguments.
    #[arg(long, requires = "acp", allow_hyphen_values = true)]
    pub acp_arg: Vec<String>,
    /// Folder for this connection's work and attachments (default: your home folder).
    #[arg(long)]
    pub workspace: Option<PathBuf>,
    /// Connect for sending requests only; skip the receiving worker.
    #[arg(long)]
    pub requester_only: bool,
    /// Keep the worker in the foreground instead of starting at sign-in.
    #[arg(long)]
    pub no_background: bool,
    /// Skip adding Litecom to the chosen agent's MCP configuration.
    #[arg(long)]
    pub no_mcp: bool,
    /// Print the sign-in link without opening a browser on this computer.
    #[arg(long)]
    pub no_browser: bool,
    #[arg(skip)]
    pub preview: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            relay: std::env::var("LITECOM_RELAY").ok(),
            connect_code: None,
            name: "This computer".into(),
            reconnect: false,
            agent: None,
            acp: None,
            acp_arg: vec![],
            workspace: None,
            requester_only: false,
            no_background: false,
            no_mcp: false,
            no_browser: false,
            preview: false,
        }
    }
}

pub fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    if !command.arg(url).status()?.success() {
        bail!("Open this address in your browser: {url}");
    }
    Ok(())
}

pub async fn setup(mut options: Setup, connection: &str) -> Result<()> {
    let path = ClientConfig::path(connection)?;
    let theme = dialoguer::theme::ColorfulTheme::default();
    println!("\n  litecom\n  Your people. Your agents. Your approval.\n");
    let mut verified_profile = None;
    let mut selected_runner = None;
    if path.exists() && !options.reconnect {
        let saved = ClientConfig::load_named(connection)?;
        let client = Client::new(saved.clone())?;
        match client.get::<Value>("/api/me").await {
            Err(error) if error.downcast_ref::<crate::client::ApiError>().is_some_and(|e|e.status==401) => {
                println!("This connection expired or was disconnected. Let's reconnect your account.");
                options.reconnect = true;
                if options.relay.is_none() { options.relay = Some(saved.relay); }
            }
            Err(error) => return Err(error.context("Your server could not be reached or did not accept this connection. Your settings are preserved; retry setup when it is available.")),
            Ok(profile) => verified_profile = Some(profile),
        }
    }
    if path.exists() && !options.reconnect {
        let saved = ClientConfig::load_named(connection)?;
        if options
            .relay
            .as_ref()
            .is_some_and(|v| v.trim_end_matches('/') != saved.relay.trim_end_matches('/'))
        {
            bail!(
                "This connection already uses {}. Use a new --connection name for another server.",
                saved.relay
            );
        }
        let profile = verified_profile
            .context("Your saved connection could not be verified. Retry setup.")?;
        println!(
            "Connected as {}.",
            profile["name"].as_str().unwrap_or("you")
        );
    } else {
        if path.exists() && options.relay.is_none() {
            options.relay = Some(ClientConfig::load_named(connection)?.relay);
        }
        let relay = match options
            .relay
            .clone()
            .or_else(|| option_env!("LITECOM_DEFAULT_RELAY").map(String::from))
        {
            Some(value) => value,
            None => dialoguer::Input::<String>::with_theme(&theme)
                .with_prompt("Litecom server address")
                .interact_text()?,
        };
        config::validate_url(&relay, true)?;
        let relay = relay.trim_end_matches('/').to_owned();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let caps: Value = http
            .get(format!("{relay}/api/capabilities"))
            .send()
            .await
            .context("Cannot reach this Litecom server. Check the address and try setup again.")?
            .error_for_status()?
            .json()
            .await?;
        if let Some(code) = &options.connect_code {
            selected_runner = Some(choose_runner(&options, connection)?);
            let token: Option<AgentToken> = http.post(format!("{relay}/device/poll"))
                .json(&json!({"device_code":code})).send().await?
                .error_for_status().context("This connection command expired or was already used. Click Connect local agent again in your workspace.")?
                .json().await?;
            let token = token.context("This connection is not authorized. Generate a new command in your signed-in workspace.")?;
            ClientConfig {
                relay,
                token: token.token,
                user_id: token.user_id,
                agent_id: token.agent_id,
                workspace: None,
                runner: None,
                approval_channel: None,
            }
            .save_named(connection)?;
            println!("Account connected.");
        } else {
            if caps["phone"] != true && caps["google"] != true {
                bail!(
                    "This server has not enabled sign-in. Ask its operator to finish setup before connecting."
                );
            }
            let channels = [("phone", "Phone"), ("slack", "Slack"), ("email", "Email")]
                .into_iter()
                .filter(|(key, _)| {
                    caps[*key] == true
                        && (caps["development"] == true || caps["direct_approvals"][*key] == true)
                })
                .collect::<Vec<_>>();
            if channels.is_empty() {
                bail!(
                    "This server has no approval channels ready. Ask its operator to finish messaging setup."
                );
            }
            let choice = dialoguer::Select::with_theme(&theme)
                .with_prompt("Where do you want to approve requests?")
                .items(channels.iter().map(|(_, label)| *label).collect::<Vec<_>>())
                .default(0)
                .interact()?;
            let channel = channels[choice].0;
            if !options.requester_only {
                selected_runner = Some(choose_runner(&options, connection)?);
            }
            let device: DeviceCode = http
                .post(format!("{relay}/device/start"))
                .json(&json!({"label":options.name}))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let url = format!("{relay}/connect/{}?approve={channel}", device.user_code);
            println!(
                "\nSign in in the browser.\nCheck this code: {}\n{url}\n",
                device.user_code
            );
            if !options.no_browser && open_browser(&url).is_err() {
                println!("Open the link above to continue.");
            }
            let start = Instant::now();
            let token: AgentToken = loop {
                if start.elapsed() > Duration::from_secs(device.expires_in as u64) {
                    bail!("The connection code expired. Run setup again.");
                }
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => bail!("Setup cancelled. Your saved settings are unchanged."),
                    _ = tokio::time::sleep(Duration::from_secs(device.interval as u64)) => {}
                }
                let response = http
                    .post(format!("{relay}/device/poll"))
                    .json(&json!({"device_code":device.device_code}))
                    .send()
                    .await?;
                if !response.status().is_success() {
                    bail!("The pairing request expired or was rejected. Run setup again.");
                }
                if let Some(token) = response.json::<Option<AgentToken>>().await? {
                    break token;
                }
            };
            let previous = ClientConfig::load_named(connection).ok();
            ClientConfig {
                relay,
                token: token.token,
                user_id: token.user_id,
                agent_id: token.agent_id,
                workspace: previous.as_ref().and_then(|c| c.workspace.clone()),
                runner: previous.and_then(|c| c.runner),
                approval_channel: Some(channel.into()),
            }
            .save_named(connection)?;
            println!("Account connected.");
        }
    }
    if options.requester_only {
        println!(
            "\nReady to ask. Try: litecom --connection {connection} ask me \"Find my launch notes\"\nMCP configuration: litecom --connection {connection} mcp-config"
        );
        return Ok(());
    }
    let mut restart_worker = options.reconnect
        || RunnerConfig::load(connection).is_err()
        || options.agent.is_some()
        || options.acp.is_some()
        || options.workspace.is_some();
    let mut settings = match selected_runner {
        Some(settings) => settings,
        None => choose_runner(&options, connection)?,
    };
    settings.save(connection)?;
    if settings.agent.is_acp() {
        let mcp = if options.no_mcp || !settings.agent.session_mcp() {
            None
        } else {
            Some(crate::model::McpConnection {
                executable: service::install_binary()?.to_string_lossy().into(),
                home: config::home()?.canonicalize()?.to_string_lossy().into(),
                connection: connection.into(),
            })
        };
        restart_worker |= serde_json::to_value(&settings.mcp)? != serde_json::to_value(&mcp)?;
        settings.mcp = mcp;
        settings.save(connection)?;
    }
    let mut saved = ClientConfig::load_named(connection)?;
    saved.workspace = Some(settings.workspace.clone());
    saved.runner = Some(settings.agent.command().into());
    saved.save_named(connection)?;
    println!(
        "\n{} · {}\nApprove a request once. Your agent does the work and sends back the result.",
        settings.agent.name(),
        settings.workspace.display()
    );
    if !options.no_mcp {
        let executable = service::install_binary()?;
        let tools = crate::agent_setup::Connection::new(connection, &executable)?;
        tools.probe().await?;
        match settings.agent {
            NativeAgent::Codex
            | NativeAgent::Claude
            | NativeAgent::Pi
            | NativeAgent::Cursor
            | NativeAgent::OpenCode
            | NativeAgent::Hermes
            | NativeAgent::OpenClaw => {
                let name = crate::agent_setup::install(&settings, &tools).await?;
                println!(
                    "Litecom tools checked and saved in {} as {name}.",
                    settings.agent.name()
                );
            }
            _ => println!("Litecom tools checked. They will be available in each ACP session."),
        }
    }
    if !options.no_background {
        println!(
            "Starting Litecom in the background. It will start again when you sign in to this computer."
        );
        let reused_worker = !restart_worker && service::status(connection).unwrap_or(false);
        if !reused_worker {
            service::install(connection)?;
        }
        // Check the actual queue heartbeat, not merely the service installation.
        let client = Client::new(saved)?;
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64
            - if reused_worker {
                crate::store::LEASE_TTL
            } else {
                2
            };
        let mut ready = false;
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if options.connect_code.is_some() {
                let state: Value = client.post("/api/connection/ready", json!({})).await?;
                if state["listening"] == true {
                    ready = true;
                    break;
                }
                continue;
            }
            let agents: Vec<crate::model::Agent> = client.get("/api/agents").await?;
            if agents.iter().any(|a| {
                a.id == client.config.agent_id && a.last_seen_at.is_some_and(|t| t >= since)
            }) {
                ready = true;
                break;
            }
        }
        if !ready {
            bail!(
                "Startup is installed, but the agent has not connected yet. Run `litecom doctor` to check it. Your setup is saved."
            );
        }
        println!("Listening for requests. You can close this terminal.");
    } else if !options.preview {
        println!("Start listening: litecom --connection {connection} worker");
    }
    if !options.preview {
        if let Some(command) = (!options.no_mcp).then(|| chat_command(&settings)).flatten() {
            println!(
                "\nReady in {}.\nStart a new chat: {command}\n\nThen ask:\n  Use Litecom to ask my agent to reply with \"Connected through Litecom.\"\n\nApprove the request in your approval channel. The reply appears in the same chat.",
                settings.agent.name()
            );
        } else {
            println!(
                "\nSend a request: litecom --connection {connection} ask me \"Reply with Connected through Litecom.\"\nTo add Litecom to another chat, get its MCP configuration: litecom --connection {connection} mcp-config"
            );
        }
    }
    Ok(())
}

/// Show an interactive chat command, never an ACP protocol server. Prefer the
/// familiar command when it is on PATH; bundled CLIs need their actual path.
fn chat_command(settings: &RunnerConfig) -> Option<String> {
    let executable = match settings.agent {
        NativeAgent::CodexAcp | NativeAgent::ClaudeAcp | NativeAgent::Acp => return None,
        NativeAgent::Pi => runner::find_executable("pi")?,
        _ => settings.executable.clone(),
    };
    let filename = executable.file_name()?;
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            directory.join(filename).canonicalize().ok()
                == executable
                    .canonicalize()
                    .ok()
                    .filter(|_| executable.is_file())
        })
    });
    let mut command = if on_path {
        filename.to_string_lossy().into_owned()
    } else if cfg!(windows) {
        format!("& '{}'", executable.to_string_lossy().replace('\'', "''"))
    } else {
        format!(
            "'{}'",
            executable.to_string_lossy().replace('\'', "'\"'\"'")
        )
    };
    if settings.agent == NativeAgent::OpenClaw {
        command.push_str(" tui");
    }
    Some(command)
}

fn choose_runner(options: &Setup, connection: &str) -> Result<RunnerConfig> {
    let theme = dialoguer::theme::ColorfulTheme::default();
    let mut saved_runner: Option<RunnerConfig> =
        match std::fs::read(RunnerConfig::path(connection)?) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .context("Your saved agent settings could not be read. Check the connection's runner configuration.")?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
    let workspace = match options
        .workspace
        .clone()
        .or_else(|| saved_runner.as_ref().map(|saved| saved.workspace.clone()))
    {
        Some(path) => path,
        None => directories::BaseDirs::new()
            .context("Cannot find your home folder. Pass --workspace /absolute/folder.")?
            .home_dir()
            .to_path_buf(),
    }
    .canonicalize()
    .context("Choose an existing folder with --workspace /absolute/folder.")?;
    if let Some(saved) = &mut saved_runner {
        saved.workspace = workspace.clone();
    }
    let settings = if let Some(saved) = saved_runner.filter(|saved| {
        options.agent.is_none() && options.acp.is_none() && saved.validate().is_ok()
    }) {
        saved
    } else {
        let detected = runner::detect();
        if options.acp.is_none()
            && (options.agent.as_deref() == Some("pi")
                || (options.agent.is_none() && detected.is_empty()))
            && let Some(hint) = runner::pi_setup_hint()
        {
            bail!("{hint}");
        }
        if detected.is_empty() && options.acp.is_none() {
            if options.connect_code.is_some() {
                bail!(
                    "No coding agents found. Install and sign in to a supported coding agent, then run this connection command again. Your workspace account is ready."
                );
            }
            bail!(
                "No coding agents found. Install and sign in to a supported coding agent, then run `litecom setup` again. Use `--requester-only` if you only want to send requests."
            );
        }
        let (agent, executable) = if let Some(command) = &options.acp {
            let path = if std::path::Path::new(&command).components().count() > 1 {
                PathBuf::from(command)
                    .canonicalize()
                    .context("Choose an installed ACP executable.")?
            } else {
                runner::find_executable(command)
                    .context("That ACP executable was not found on PATH.")?
            };
            (NativeAgent::Acp, path)
        } else if let Some(requested) = &options.agent {
            detected
                .into_iter()
                .find(|(a, _)| a.command() == requested.as_str())
                .context("That agent was not found. Install it and run setup again.")?
        } else if detected.len() == 1 {
            println!("Using {}.", detected[0].0.name());
            detected[0].clone()
        } else {
            let names = detected.iter().map(|(a, _)| a.name()).collect::<Vec<_>>();
            let choice = dialoguer::Select::with_theme(&theme)
                .with_prompt("Which agent should handle your requests?")
                .items(&names)
                .default(0)
                .interact()
                .context(
                    "Choose an agent in a terminal, or add --agent <name> to the setup command.",
                )?;
            detected[choice].clone()
        };
        RunnerConfig {
            agent,
            executable,
            workspace,
            arguments: if agent == NativeAgent::Acp {
                options.acp_arg.clone()
            } else {
                agent.default_arguments()
            },
            mcp: None,
        }
    };
    settings.validate()?;
    Ok(settings)
}
