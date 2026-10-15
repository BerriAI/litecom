//! One-command local evaluation; no installed services or harness config changes.
use crate::{
    api::{AppState, router},
    client::Client,
    config::{self, ClientConfig},
    onboarding::{self, Setup},
    providers::Providers,
    runner::{self, RunnerConfig},
    store::Store,
};
use anyhow::{Context, Result, bail};

pub async fn run(no_browser: bool) -> Result<()> {
    if runner::detect().is_empty() {
        if let Some(hint) = runner::pi_setup_hint() {
            bail!(
                "{}",
                hint.replace("litecom setup --agent pi", "litecom try")
            );
        }
        bail!(
            "No coding agents found. Install and sign in to Codex or Claude Code, then run `litecom try` again."
        );
    }
    let temp = tempfile::Builder::new()
        .prefix("litecom-preview-")
        .tempdir()?;
    config::set_home(temp.path().join("config"))?;
    let folder = temp.path().join("example");
    config::private_dir(&folder)?;
    config::write_private(
        &folder.join("welcome.txt"),
        b"Hello from Litecom.\nYou approved this exact file before downloading it.\n",
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let data = temp.path().join("relay");
    let key = config::load_key(&data)?;
    let state = AppState::new(
        Store::open(data.join("litecom.db"), key)?,
        origin.clone(),
        Providers::development()?,
    );
    let mut services = tokio::task::JoinSet::<Result<()>>::new();
    let notifications = state.clone();
    services.spawn(async move {
        crate::notifications::run(notifications).await;
        Ok(())
    });
    services.spawn(async move {
        axum::serve(listener, router(state))
            .await
            .context("The local preview server stopped.")
    });
    println!(
        "\nLocal preview · keep this terminal open.\nChoose Phone and use +14155550123 as a test number. Codes print here; no messages are sent.\nThe preview uses a sample folder and your signed-in agent's normal billing after you approve a run.\n"
    );
    onboarding::setup(
        Setup {
            connect_code: None,
            relay: Some(origin.clone()),
            name: "Local preview".into(),
            reconnect: false,
            agent: None,
            acp: None,
            acp_arg: vec![],
            workspace: Some(folder),
            requester_only: false,
            no_background: true,
            no_mcp: true,
            no_browser,
            preview: true,
        },
        "default",
    )
    .await?;
    let client = Client::new(ClientConfig::load_named("default")?)?;
    let settings = RunnerConfig::load("default")?;
    let task = client
        .ask(
            "me",
            None,
            "Read welcome.txt, briefly explain it, and attach the file.",
            &crate::crypto::id(),
        )
        .await?;
    let url = format!("{origin}/request/{}", task.id);
    println!(
        "\nYour first request is ready.\n{url}\n\nApprove → your agent does the work → download welcome.txt.\nPress Ctrl+C to stop. Preview accounts and files are removed on exit.\n"
    );
    if !no_browser {
        let _ = onboarding::open_browser(&url);
    }
    tokio::select! {
        result=runner::run(client,settings,false)=>result?,
        result=services.join_next()=>{
            result.context("The preview server stopped.")???;
        },
        _=tokio::signal::ctrl_c()=>println!("\nPreview stopped."),
    }
    services.shutdown().await;
    Ok(())
}
