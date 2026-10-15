use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use litecom::{
    api::{AppState, router},
    client::Client,
    config::{self, ClientConfig},
    model::{Contact, TaskView},
    onboarding::{Setup, open_browser},
    providers::Providers,
    runner::RunnerConfig,
    store::Store,
};
use serde_json::{Value, json};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "litecom",
    version,
    about = "Ask someone's agent. Review the response. Get the result.",
    after_help = "Run litecom with no command to start guided setup."
)]
struct Cli {
    /// Store this connection in a different private configuration folder.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Use a separate, independently revocable connection for this harness.
    #[arg(
        long,
        global = true,
        default_value = "default",
        env = "LITECOM_CONNECTION"
    )]
    connection: String,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Try the complete approval and download flow locally in one terminal.
    Try {
        #[arg(long)]
        no_browser: bool,
    },
    /// Connect this computer or harness to your account.
    Setup(Setup),
    /// Listen for requests and run approved work with your chosen agent.
    Worker {
        /// Handle at most one queued item, then exit.
        #[arg(long)]
        once: bool,
    },
    /// Manage automatic background startup for this connection.
    Service {
        #[command(subcommand)]
        action: ServiceCommand,
    },
    /// Remove this computer’s Litecom connections, startup tasks and installation.
    Uninstall {
        /// Remove every local connection without showing the picker.
        #[arg(long)]
        all: bool,
    },
    /// Run a self-hosted Litecom relay.
    Relay {
        #[arg(long, default_value = "127.0.0.1:4141")]
        listen: SocketAddr,
        #[arg(long)]
        public_url: String,
        #[arg(long)]
        data: Option<PathBuf>,
        #[arg(
            long,
            help = "Local-only test delivery. Codes print in this terminal; no messages are sent."
        )]
        dev: bool,
    },
    /// Snapshot a relay database and its encryption key into a new folder.
    Backup {
        #[arg(long)]
        data: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Check a backup's checksums, database integrity, and encryption key.
    VerifyBackup { path: PathBuf },
    /// Recover data into a new folder; disconnect old access and stop pending work.
    Restore {
        path: PathBuf,
        #[arg(long)]
        data: PathBuf,
    },
    /// Ask a person, or use 'me' for your own agents.
    Ask {
        contact: String,
        request: String,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Show your shared request inbox.
    Inbox {
        #[arg(long, requires = "before_id")]
        before_time: Option<i64>,
        #[arg(long, requires = "before_time")]
        before_id: Option<String>,
    },
    /// Read one request and its approved result.
    Status { id: String },
    /// Show your contacts or your contact invite link.
    Contacts {
        #[arg(long)]
        invite: bool,
    },
    /// Download an approved file and verify its contents.
    Download {
        task: String,
        file: String,
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// Acknowledge receipt of a response.
    Receive { id: String },
    /// Cancel work or revoke your shared response.
    Cancel { id: String },
    /// Serve MCP over standard input/output for a coding harness.
    Mcp,
    /// Print MCP configuration without including credentials.
    McpConfig,
    /// Check your connection and explain what is ready.
    Doctor,
    /// Open the review inbox in your browser.
    Open,
}
#[derive(Subcommand)]
enum ServiceCommand {
    /// Install or restart background startup.
    Start,
    /// Stop the current worker.
    Stop,
    /// Show whether the background worker is running.
    Status,
    /// Remove automatic startup, preserving your account and settings.
    Remove,
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if let Err(e) = run(Cli::parse()).await {
        eprintln!("\nLitecom: {e:#}");
        std::process::exit(1);
    }
}
async fn run(cli: Cli) -> Result<()> {
    let command = cli
        .command
        .unwrap_or_else(|| Command::Setup(Setup::default()));
    if let Command::Try { no_browser } = &command {
        return litecom::preview::run(*no_browser).await;
    }
    if let Some(home) = cli.home {
        config::set_home(home)?;
    }
    match command {
        Command::Relay {
            listen,
            public_url,
            data,
            dev,
        } => {
            let url = config::validate_url(&public_url, dev)?;
            if dev
                && (!listen.ip().is_loopback()
                    || !matches!(
                        url.host_str(),
                        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
                    ))
            {
                bail!("Development delivery is only allowed on loopback addresses.");
            }
            let data = data.unwrap_or(config::home()?.join("relay"));
            let key = config::load_key(&data)?;
            let state = AppState::new(
                Store::open(data.join("litecom.db"), key)?,
                public_url.clone(),
                Providers::from_env(dev)?,
            );
            let listener = tokio::net::TcpListener::bind(listen).await?;
            let notifications = tokio::spawn(litecom::notifications::run(state.clone()));
            let email_replies = tokio::spawn(litecom::email_replies::run(state.clone()));
            let cloud = tokio::spawn(litecom::cloud::run(state.clone()));
            eprintln!(
                "Litecom relay · {public_url}\nData: {}{}",
                data.display(),
                if dev {
                    "\nLOCAL DEVELOPMENT: no real messages are delivered."
                } else {
                    ""
                }
            );
            axum::serve(listener, router(state))
                .with_graceful_shutdown(shutdown())
                .await?;
            notifications.abort();
            email_replies.abort();
            cloud.abort();
        }
        Command::Backup { data, output } => {
            litecom::backup::create(&data, &output)?;
            println!(
                "Backup verified: {}\nKeep this folder private: it includes the key that decrypts your data.",
                output.display()
            );
        }
        Command::VerifyBackup { path } => {
            let manifest = litecom::backup::verify(&path)?;
            println!(
                "Backup verified · Litecom {} · created at {}",
                manifest.version, manifest.created_at
            );
        }
        Command::Restore { path, data } => {
            litecom::backup::restore(&path, &data)?;
            println!(
                "Recovered data: {}\nSign in again, re-pair agents, reconnect contacts and approval channels. Previous download access is revoked and pending work will not resume. Inspect existing cloud sessions directly before sending work again.",
                data.display()
            );
        }
        Command::Uninstall { all } => litecom::uninstall::run(all).await?,
        Command::Setup(options) => litecom::onboarding::setup(options, &cli.connection).await?,
        Command::Service { action } => match action {
            ServiceCommand::Start => litecom::service::install(&cli.connection)?,
            ServiceCommand::Stop => litecom::service::stop(&cli.connection)?,
            ServiceCommand::Remove => litecom::service::remove(&cli.connection)?,
            ServiceCommand::Status => println!(
                "{}",
                if litecom::service::status(&cli.connection)? {
                    "Background worker is running."
                } else {
                    "Background worker is stopped. Run `litecom service start`."
                }
            ),
        },
        Command::Worker { once } => {
            let client = Client::new(ClientConfig::load_named(&cli.connection)?)?;
            let settings = RunnerConfig::load(&cli.connection)?;
            tokio::select! {
                result = litecom::runner::run(client, settings, once) => result?,
                _ = shutdown() => eprintln!("Litecom stopped. Running actions may have partial effects; they will not be repeated automatically."),
            }
        }
        Command::McpConfig => {
            let exe = std::env::current_exe()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"mcpServers":{"litecom":{"command":exe,"args":["--home",config::home()?,"--connection",cli.connection,"mcp"]}}})
                )?
            );
        }
        other => {
            let client = Client::new(ClientConfig::load_named(&cli.connection)?)?;
            match other {
                Command::Ask {
                    contact,
                    request,
                    agent,
                    request_id,
                } => {
                    let task = client
                        .ask(
                            &contact,
                            agent.as_deref(),
                            &request,
                            &request_id.unwrap_or_else(Client::request_id),
                        )
                        .await?;
                    println!(
                        "Request saved · {}\n{}",
                        task.status.as_str(),
                        client.review_url(&task.id)
                    );
                }
                Command::Inbox {
                    before_time,
                    before_id,
                } => {
                    let tasks = client.inbox(before_time, before_id).await?;
                    if tasks.len() == 100
                        && let Some(last) = tasks.last()
                    {
                        println!(
                            "Older requests: repeat this command with --before-time {} --before-id {}\n",
                            last.created_at, last.id
                        );
                    }
                    if tasks.is_empty() {
                        if before_time.is_some() {
                            println!("No older requests.");
                        } else {
                            println!(
                                "No requests yet. Try: litecom ask me \"Find my launch notes\""
                            );
                        }
                    }
                    for t in tasks {
                        println!(
                            "{}  {}\n  {}\n",
                            t.status.as_str(),
                            t.request,
                            client.review_url(&t.id)
                        );
                    }
                }
                Command::Status { id } => {
                    let task: TaskView = client.get(&format!("/api/tasks/{id}")).await?;
                    print_task(&task);
                }
                Command::Contacts { invite } => {
                    if invite {
                        let p: Value = client.get("/api/me").await?;
                        println!(
                            "{}/invite/{}",
                            client.config.relay.trim_end_matches('/'),
                            p["invite"]
                                .as_str()
                                .context("The server omitted your contact link.")?
                        );
                    } else {
                        let contacts: Vec<Contact> = client.get("/api/contacts").await?;
                        for c in contacts {
                            println!("{} · {}\n  {}", c.name, c.status, c.id);
                        }
                    }
                }
                Command::Download { task, file, to } => {
                    let path = client
                        .download(
                            &task,
                            &file,
                            &to.unwrap_or(config::home()?.join("downloads").join(&task)),
                        )
                        .await?;
                    println!("Saved and verified: {}", path.display());
                }
                Command::Receive { id } => {
                    let t: TaskView = client
                        .post(&format!("/api/tasks/{id}/received"), json!({}))
                        .await?;
                    print_task(&t);
                }
                Command::Cancel { id } => {
                    let t: TaskView = client
                        .post(&format!("/api/tasks/{id}/cancel"), json!({}))
                        .await?;
                    print_task(&t);
                }
                Command::Mcp => litecom::mcp::serve(client).await?,
                Command::Doctor => {
                    let p: Value = client.get("/api/me").await?;
                    let agents: Value = client.get("/api/agents").await?;
                    match RunnerConfig::load(&cli.connection) {
                        Ok(settings) => println!(
                            "Main agent: {}\nFolder: {}\nBackground worker: {}",
                            settings.agent.name(),
                            settings.workspace.display(),
                            if litecom::service::status(&cli.connection).unwrap_or(false) {
                                "running"
                            } else {
                                "stopped; run `litecom service start` or `litecom worker`"
                            }
                        ),
                        Err(error) => println!("Receiving agent needs setup: {error}"),
                    }
                    println!(
                        "Connected as {}\nRelay: {}\nAgent connection: {}\n{}",
                        p["name"].as_str().unwrap_or("you"),
                        client.config.relay,
                        client.config.agent_id,
                        serde_json::to_string_pretty(&agents)?
                    );
                }
                Command::Open => open_browser(&client.config.relay)?,
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}
fn print_task(task: &TaskView) {
    println!("{} · {}\n", task.status.as_str(), task.id);
    if let Some(response) = &task.response {
        println!("{}", response.text);
        for file in &response.attachments {
            println!("  {} · {} bytes · {}", file.name, file.size, file.id);
        }
    }
    if let Some(receipt) = &task.receipt {
        println!("{}", receipt.summary);
    }
    if task.received_at.is_some() {
        println!("Received.");
    }
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("signal handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
