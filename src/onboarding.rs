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
    /// Folder your agent can use for this connection's work and attachments.
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
    if path.exists() && !options.reconnect {
        let saved = ClientConfig::load_named(connection)?;
        let client = Client::new(saved.clone())?;
        match client.get::<Value>("/api/me").await {
            Err(error) if error.downcast_ref::<crate::client::ApiError>().is_some_and(|e|e.status==401) => {
                println!("This connection expired or was disconnected. Let's reconnect your phone account.");
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
        let choice = dialoguer::Select::with_theme(&theme)
            .with_prompt("Where do you want to approve requests?")
            .items(["Phone", "Slack", "Email"])
            .default(0)
            .interact()?;
        let relay = match options
            .relay
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
        let channel = ["phone", "slack", "email"][choice];
        if caps[channel] != true {
            bail!(
                "{} approvals are not available on this server. Run setup again and choose an available channel. Nothing was changed.",
                ["Phone", "Slack", "Email"][choice]
            );
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
            "\nConnect with your phone in the browser.\nCheck this code: {}\n{url}\n",
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
        println!("Phone account connected.");
    }
    if options.requester_only {
        println!(
            "\nReady to ask. Try: litecom --connection {connection} ask me \"Find my launch notes\"\nMCP configuration: litecom --connection {connection} mcp-config"
        );
        return Ok(());
    }
    let saved_runner = RunnerConfig::load(connection).ok();
    let mut restart_worker = options.reconnect
        || saved_runner.is_none()
        || options.agent.is_some()
        || options.acp.is_some()
        || options.workspace.is_some();
    let mut settings = if let Some(mut saved) =
        saved_runner.filter(|_| options.agent.is_none() && options.acp.is_none())
    {
        if let Some(path) = options.workspace {
            saved.workspace = path
                .canonicalize()
                .context("Choose an existing working folder.")?;
            saved.save(connection)?;
        }
        saved
    } else {
        let detected = runner::detect();
        if options.acp.is_none()
            && (options.agent.as_deref() == Some("pi")
                || (options.agent.is_none() && detected.is_empty()))
            && let Some(hint) = runner::pi_setup_hint()
        {
            bail!("{hint} Your phone account is saved.");
        }
        if detected.is_empty() && options.acp.is_none() {
            bail!(
                "No coding agents found. Install and sign in to Codex or Claude Code, then run `litecom setup` again. Your phone account is saved; you can already send requests with `litecom ask`."
            );
        }
        let (agent, executable) = if let Some(command) = options.acp {
            let path = if std::path::Path::new(&command).components().count() > 1 {
                PathBuf::from(command)
                    .canonicalize()
                    .context("Choose an installed ACP executable.")?
            } else {
                runner::find_executable(&command)
                    .context("That ACP executable was not found on PATH.")?
            };
            (NativeAgent::Acp, path)
        } else if let Some(requested) = options.agent {
            detected
                .into_iter()
                .find(|(a, _)| a.command() == requested)
                .context("That agent was not found. Install it and run setup again.")?
        } else {
            let names = detected.iter().map(|(a, _)| a.name()).collect::<Vec<_>>();
            let choice = dialoguer::Select::with_theme(&theme)
                .with_prompt("Which agent should handle your requests?")
                .items(&names)
                .default(0)
                .interact()?;
            detected[choice].clone()
        };
        let workspace = match options.workspace {
            Some(path) => path,
            None => PathBuf::from(
                dialoguer::Input::<String>::with_theme(&theme)
                    .with_prompt("Folder your agent can work in")
                    .default(std::env::current_dir()?.to_string_lossy().into())
                    .interact_text()?,
            ),
        }
        .canonicalize()
        .context("Choose an existing folder. Your phone connection has been saved.")?;
        let settings = RunnerConfig {
            agent,
            executable,
            workspace,
            arguments: if agent == NativeAgent::Acp {
                options.acp_arg
            } else {
                agent.default_arguments()
            },
            mcp: None,
        };
        settings.save(connection)?;
        settings
    };
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
        "\n{} · {}\nEach request asks before launching your agent and before sharing its result.",
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
                    "Litecom tools checked and saved in {} as {name}. Start a new agent session to use them.",
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
        if restart_worker || !service::status(connection).unwrap_or(false) {
            service::install(connection)?;
        }
        // Check the actual queue heartbeat, not merely the service installation.
        let client = Client::new(saved)?;
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64
            - 2;
        let mut ready = false;
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_secs(1)).await;
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
        println!(
            "\nTry: litecom --connection {connection} ask me \"Find my launch notes\"\nAdd a person: litecom --connection {connection} contacts --invite"
        );
    }
    Ok(())
}
