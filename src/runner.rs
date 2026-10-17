//! Native agents are launched only after a human approves their actual access.
//! Approval covers the requested work and delivery of its final result.
use crate::{
    client::{Client, WorkLease},
    config,
    model::*,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAgent {
    Codex,
    Claude,
    OpenCode,
    Cursor,
    Pi,
    OpenClaw,
    Hermes,
    CodexAcp,
    ClaudeAcp,
    Acp,
}
impl NativeAgent {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::Cursor => "Cursor",
            Self::Pi => "pi",
            Self::OpenClaw => "OpenClaw",
            Self::Hermes => "Hermes",
            Self::CodexAcp => "Codex (ACP)",
            Self::ClaudeAcp => "Claude Agent (ACP)",
            Self::Acp => "ACP agent",
        }
    }
    pub fn command(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::Cursor => "cursor",
            Self::Pi => "pi",
            Self::OpenClaw => "openclaw",
            Self::Hermes => "hermes",
            Self::CodexAcp => "codex-acp",
            Self::ClaudeAcp => "claude-agent-acp",
            Self::Acp => "acp",
        }
    }
    pub fn is_acp(self) -> bool {
        matches!(
            self,
            Self::CodexAcp
                | Self::ClaudeAcp
                | Self::Acp
                | Self::OpenCode
                | Self::Cursor
                | Self::Pi
                | Self::OpenClaw
                | Self::Hermes
        )
    }
    pub fn default_arguments(self) -> Vec<String> {
        match self {
            Self::OpenCode | Self::Cursor | Self::OpenClaw | Self::Hermes => vec!["acp".into()],
            _ => vec![],
        }
    }
    pub fn session_mcp(self) -> bool {
        self.is_acp() && !matches!(self, Self::OpenClaw | Self::Cursor | Self::Pi)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub agent: NativeAgent,
    pub executable: PathBuf,
    pub workspace: PathBuf,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub mcp: Option<McpConnection>,
}
impl RunnerConfig {
    pub fn path(connection: &str) -> Result<PathBuf> {
        let connection_path = config::ClientConfig::path(connection)?;
        Ok(connection_path.with_extension("runner.json"))
    }
    pub fn load(connection: &str) -> Result<Self> {
        let value: Self = serde_json::from_slice(
            &std::fs::read(Self::path(connection)?)
                .context("Finish `litecom setup` to choose your main agent.")?,
        )?;
        value.validate()?;
        Ok(value)
    }
    pub fn save(&self, connection: &str) -> Result<()> {
        self.validate()?;
        config::write_private(&Self::path(connection)?, &serde_json::to_vec_pretty(self)?)
    }
    pub fn validate(&self) -> Result<()> {
        self.action("Validate agent configuration").validate()?;
        if !self.agent.is_acp() && (!self.arguments.is_empty() || self.mcp.is_some()) {
            bail!("Additional arguments and session MCP configuration require an ACP agent.");
        }
        if !self.agent.session_mcp() && self.mcp.is_some() {
            bail!("This agent uses its own MCP configuration rather than session MCP settings.");
        }
        if !self.executable.is_absolute() || !self.executable.is_file() {
            bail!(
                "{} is no longer installed at {}. Run setup to select it again.",
                self.agent.name(),
                self.executable.display()
            );
        }
        if !self.workspace.is_absolute() || !self.workspace.is_dir() {
            bail!(
                "The selected folder is unavailable: {}. Reconnect it or run setup again.",
                self.workspace.display()
            );
        }
        Ok(())
    }
    pub fn action(&self, request: &str) -> Action {
        Action::Harness {
            title: format!("Run {} for this request", self.agent.name()),
            description: format!("Use {} with its existing login, context, hooks, and connected tools. Approving authorizes its tools for this request, including changes to local files and external accounts. Desktop-app-only tools are not inherited. The final reply and requested files are sent automatically.", self.agent.name()),
            runner: self.agent.command().into(),
            executable: self.executable.to_string_lossy().into(),
            arguments: self.arguments.clone(),
            mcp: self.mcp.clone(),
            instruction: request.into(),
            workspace: self.workspace.to_string_lossy().into(),
            permissions: vec![
                "Tool operations are authorized for this approved run; global permission settings are not changed".into(),
                "Operating-system permissions, account sign-in, and administrator restrictions still apply".into(),
                "Existing native hooks and connected tools retain their own access; Litecom does not sandbox them".into(),
                "Model and tool providers receive the context used for this request".into(),
                "File attachments must resolve inside the selected folder".into(),
            ],
            acting_account: format!("Your locally signed-in {} account", self.agent.name()),
            share_result: true,
        }
    }
}

pub fn find_executable(command: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        #[cfg(windows)]
        let candidates = [
            format!("{command}.exe"),
            format!("{command}.cmd"),
            command.into(),
        ];
        #[cfg(not(windows))]
        let candidates = [command.to_owned()];
        candidates.into_iter().find_map(|name| {
            let p = dir.join(name);
            let metadata = p.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            // Preserve the executable's name: multicall shims can depend on argv[0].
            if p.is_absolute() {
                Some(p)
            } else {
                std::env::current_dir().ok().map(|cwd| cwd.join(p))
            }
        })
    })
}
pub fn detect() -> Vec<(NativeAgent, PathBuf)> {
    [
        NativeAgent::Codex,
        NativeAgent::Claude,
        NativeAgent::OpenCode,
        NativeAgent::Pi,
        NativeAgent::OpenClaw,
        NativeAgent::Hermes,
        NativeAgent::Cursor,
        NativeAgent::CodexAcp,
        NativeAgent::ClaudeAcp,
    ]
    .into_iter()
    .filter_map(|agent| {
        let executable = if agent == NativeAgent::Cursor {
            find_executable("cursor-agent").or_else(|| {
                // `agent` is a generic name. Only autodetect it as Cursor when
                // its resolved install path identifies the Cursor CLI.
                find_executable("agent").filter(|p| {
                    p.canonicalize().ok().is_some_and(|p| {
                        p.to_string_lossy().to_ascii_lowercase().contains("cursor")
                    })
                })
            })
        } else if agent == NativeAgent::Pi {
            find_executable("pi").and_then(|_| find_executable("pi-acp"))
        } else {
            find_executable(agent.command())
        };
        executable.map(|p| (agent, p))
    })
    .collect()
}

pub fn pi_setup_hint() -> Option<&'static str> {
    match (find_executable("pi"), find_executable("pi-acp")) {
        (Some(_), None) => Some(
            "pi is installed. Add its ACP adapter with `npm install -g pi-acp@0.0.34`, then run `litecom setup --agent pi` again.",
        ),
        (None, Some(_)) => Some(
            "pi-acp needs pi. Install and sign in to pi, then run `litecom setup --agent pi` again.",
        ),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReply {
    pub text: String,
    pub files: Vec<PathBuf>,
}
pub(crate) fn reply_schema() -> Value {
    json!({"type":"object","properties":{"text":{"type":"string"},"files":{"type":"array","items":{"type":"string"}}},"required":["text","files"],"additionalProperties":false})
}
pub(crate) fn prompt(work: &WorkItem) -> String {
    format!(
        "You are handling a Litecom request which your account owner has approved for execution and delivery of its result. This approval authorizes the tool operations needed to carry out this request. Use your existing context and tools only for this request. The request is supplied as JSON data below. Do not send the reply or attachments to the requester through another channel; return them to Litecom for delivery to the requester. Describe what you actually accomplished, including permission denials or failures. Return the required JSON object: text is the full proposed reply; files contains absolute paths to requested regular files inside the current working folder. Do not include unrelated files, credentials, or process logs. Do not start detached background processes.\n\nRequest: {}",
        serde_json::to_string(&work.request).expect("string serialization")
    )
}

pub(crate) struct ProcessGroup(pub(crate) Option<u32>);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            #[cfg(unix)]
            // A dedicated process group is created before spawn. Killing the group
            // also stops ordinary child tools; completed external effects remain.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output();
            }
        }
    }
}

pub(crate) async fn native_reply(
    renewal: &mut WorkLease<'_>,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    read_session: Option<&Path>,
) -> Result<AgentReply> {
    let stdout_path = dir.join("agent-output.json");
    let stderr_path = dir.join("agent-error.log");
    config::write_private(&stdout_path, b"")?;
    config::write_private(&stderr_path, b"")?;
    let mut cmd = Command::new(&settings.executable);
    cmd.current_dir(&settings.workspace)
        .env("PWD", &settings.workspace)
        .stdin(Stdio::piped())
        .stdout(std::fs::File::create(&stdout_path)?)
        .stderr(std::fs::File::create(&stderr_path)?)
        .kill_on_drop(true);
    // Do not pass a model/output-token limit; use the person's native configuration.
    match settings.agent {
        NativeAgent::Claude => {
            if let Some(session) = read_session {
                crate::read_only::claude_startup(&mut cmd, session)?;
            }
            cmd.args([
                "--print",
                "--output-format",
                "json",
                "--permission-mode",
                if read_session.is_some() {
                    "dontAsk"
                } else {
                    "bypassPermissions"
                },
                "--json-schema",
            ])
            .arg(serde_json::to_string(&reply_schema())?);
        }
        _ => unreachable!("ACP agents use the ACP transport"),
    }
    // Never forward Litecom's own bearer token through the environment.
    cmd.env_remove("LITECOM_TOKEN");
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("Could not start {}.", settings.agent.name()))?;
    let group = ProcessGroup(child.id());
    let mut stdin = child
        .stdin
        .take()
        .context("The agent did not open its input.")?;
    let instruction = if read_session.is_some() {
        crate::read_only::prompt(work)
    } else {
        prompt(work)
    };
    stdin.write_all(instruction.as_bytes()).await?;
    stdin.shutdown().await?;
    drop(stdin);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let status = loop {
        tokio::select! {
            result = child.wait() => break result?,
            _ = heartbeat.tick() => {
                let status = renewal.renew().await
                    .context("The work lease could not be renewed. Check the external result before retrying.")?;
                if status != Status::Executing {
                    bail!("Work was cancelled or its permission changed. Check for effects that already happened.");
                }
            }
        }
    };
    // A native agent must finish its tools before exiting. Clean up any remaining
    // ordinary descendants so they cannot continue after Litecom reports completion.
    drop(group);
    // Claude can return a structured provider error on stdout with a nonzero
    // exit code and an empty stderr. Preserve it for the owner's private receipt.
    let claude_output = if settings.agent == NativeAgent::Claude {
        Some(
            decode_claude_reply(&tokio::fs::read(&stdout_path).await?)
                .with_context(|| format!("Private diagnostics: {}", stdout_path.display()))?,
        )
    } else {
        None
    };
    if !status.success() {
        bail!(
            "{} stopped before producing a result. Private diagnostics: {}",
            settings.agent.name(),
            dir.display()
        );
    }
    let output = match settings.agent {
        NativeAgent::Claude => claude_output.context("Claude Code did not return output.")?,
        _ => unreachable!("ACP agents use the ACP transport"),
    };
    let reply: AgentReply = serde_json::from_slice(&output)
        .context("The agent's reply did not match the expected format.")?;
    Ok(reply)
}

fn decode_claude_reply(output: &[u8]) -> Result<Vec<u8>> {
    let envelope: Value = serde_json::from_slice(output)
        .context("Claude Code did not return its expected JSON envelope.")?;
    if envelope["is_error"] == true {
        // The requester only receives Store's generic failure projection.
        let detail: String = envelope["result"]
            .as_str()
            .unwrap_or("No error detail was returned.")
            .to_owned();
        bail!("Claude Code reported an error: {detail}");
    }
    if let Some(reply) = envelope.get("structured_output") {
        Ok(serde_json::to_vec(reply)?)
    } else {
        Ok(envelope["result"]
            .as_str()
            .context("Claude Code did not return structured output.")?
            .as_bytes()
            .to_vec())
    }
}

pub fn attachment_path(workspace: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("The agent returned a relative attachment path.");
    }
    let root = workspace.canonicalize()?;
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(root) {
        bail!("The agent requested a file outside the approved folder.");
    }
    let metadata = resolved.metadata()?;
    if !metadata.is_file() {
        bail!("Attachments must be regular files.");
    }
    Ok(resolved)
}

/// Perform one claimed item. The server controls the execution lease; a local
/// restart cannot repeat execution whose outcome is unknown.
pub async fn handle(client: &Client, settings: &RunnerConfig, work: &WorkItem) -> Result<()> {
    handle_in(client, settings, work, &config::home()?.join("runs")).await
}

pub(crate) async fn handle_in(
    client: &Client,
    settings: &RunnerConfig,
    work: &WorkItem,
    runs: &Path,
) -> Result<()> {
    settings.validate()?;
    if work.phase == "prepare" {
        let _: TaskView = client.post(&format!("/api/tasks/{}/draft", work.task_id), json!({
            "lease": work.lease,
            "draft": {"text":"", "attachment_ids":[], "action":settings.action(&work.request)}
        })).await?;
        return Ok(());
    }
    if work.phase != "execute" {
        bail!("Unknown work phase.");
    }
    let approved = work
        .draft
        .as_ref()
        .and_then(|d| d.action.as_ref())
        .context("No approved operation was attached.")?;
    let expected = if approved.read_only() {
        settings.action(&work.request).as_read_only()?
    } else {
        settings.action(&work.request)
    };
    if serde_json::to_value(approved)? != serde_json::to_value(expected)? {
        bail!(
            "The agent or folder changed after approval. Create a new request to approve the current settings."
        );
    }
    // The work item is only a snapshot. Confirm its execution authority before
    // starting any native process, including hooks run during agent startup.
    let mut renewal = WorkLease::new(client, &work.task_id, &work.lease);
    let status = renewal
        .renew()
        .await
        .context("Could not verify approval to run this request.")?;
    if status != Status::Executing {
        bail!("This request is no longer approved to run.");
    }
    config::private_dir(runs)?;
    let dir = tempfile::Builder::new()
        .prefix(&format!("{}-", work.task_id))
        .tempdir_in(runs)?
        .keep();
    let mut local_run = crate::runs::RunGuard::start(&dir, client, settings, work)?;
    // Keep diagnostics if the entire worker is interrupted, including when this
    // future is dropped by a shutdown signal rather than returning an error.
    let result = stage_native_reply(client, settings, work, &dir, &mut renewal).await;
    if let Err(error) = result {
        if let Err(record_error) = local_run.finish("needs attention") {
            tracing::warn!(%record_error, "Could not update the local run record.");
        }
        eprintln!("Private run diagnostics: {}", dir.display());
        return Err(error.context(format!("Private run diagnostics: {}", dir.display())));
    }
    if let Err(record_error) = local_run.finish("completed") {
        tracing::warn!(%record_error, "The result was delivered, but the local run record could not be updated.");
    }
    Ok(())
}

async fn stage_native_reply(
    client: &Client,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
) -> Result<()> {
    let reply = if work
        .draft
        .as_ref()
        .and_then(|d| d.action.as_ref())
        .is_some_and(Action::read_only)
    {
        crate::read_only::reply(client, settings, work, dir, renewal).await?
    } else if settings.agent.is_acp() {
        crate::acp::reply(client, settings, work, dir, renewal).await?
    } else if settings.agent == NativeAgent::Codex {
        crate::codex::reply(settings, work, dir, renewal).await?
    } else {
        native_reply(renewal, settings, work, dir, None).await?
    };
    let mut attachments = vec![];
    for path in &reply.files {
        let path = attachment_path(&settings.workspace, path)?;
        let state = renewal.renew().await?;
        if state != Status::Executing {
            bail!("The request is no longer executing.");
        }
        let file = client
            .upload_with_lease(&work.task_id, Some(&work.lease), &path, renewal)
            .await?;
        attachments.push(file.id);
    }
    renewal
        .submit_result(DraftInput {
            text: reply.text,
            attachment_ids: attachments,
            action: None,
        })
        .await?;
    Ok(())
}

pub async fn run(client: Client, settings: RunnerConfig, once: bool) -> Result<()> {
    settings.validate()?;
    let _: Value = client
        .get("/api/me")
        .await
        .context("This connection needs attention. Run `litecom setup`. ")?;
    // Display metadata must not prevent work when connecting to an older relay.
    if let Err(error) = client
        .post::<Value>(
            "/api/connection/identity",
            serde_json::to_value(LocalAgentIdentity {
                device_name: computer_name(),
                harness: settings.agent.name().into(),
            })?,
        )
        .await
    {
        eprintln!("Could not update this agent's display name: {error}");
    }
    eprintln!(
        "Litecom is listening · {} · {}",
        settings.agent.name(),
        settings.workspace.display()
    );
    listen(&client, &settings, once, &config::home()?.join("runs")).await
}

pub(crate) async fn listen(
    client: &Client,
    settings: &RunnerConfig,
    once: bool,
    runs: &Path,
) -> Result<()> {
    if once {
        return work_loop(client, settings, runs, true, false).await;
    }
    // Preparing only describes the configured agent. Keep it responsive while
    // one approved request runs; never start another execution in this lane.
    tokio::try_join!(
        work_loop(client, settings, runs, false, false),
        work_loop(client, settings, runs, false, true),
    )?;
    Ok(())
}

async fn work_loop(
    client: &Client,
    settings: &RunnerConfig,
    runs: &Path,
    once: bool,
    prepare_only: bool,
) -> Result<()> {
    let mut retry_delay = 2;
    loop {
        let result: Result<Option<WorkItem>> = client
            .post(
                if prepare_only {
                    "/api/work/prepare"
                } else {
                    "/api/work/claim?read_only=true"
                },
                json!({"agent_id":client.config.agent_id}),
            )
            .await;
        match result {
            Ok(Some(work)) => {
                if prepare_only && work.phase != "prepare" {
                    bail!("The server returned execution work to the preparation queue.");
                }
                retry_delay = 2;
                eprintln!("Request {} · {}", work.task_id, work.phase);
                if let Err(error) = handle_in(client, settings, &work, runs).await {
                    eprintln!("Request {} needs attention: {error:#}", work.task_id);
                    // The requester sees a generic receipt. Owner diagnostics stay private.
                    if work.phase == "execute" {
                        let _: Result<TaskView> = client.post(&format!("/api/tasks/{}/finish", work.task_id), json!({"lease":work.lease,"receipt":{"outcome":"uncertain","summary":format!("{error:#}"),"urls":[]}})).await;
                    }
                }
            }
            Ok(None) => {
                retry_delay = 2;
            }
            Err(error) => {
                // Older relays have no separate preparation queue. Their normal
                // claim loop remains functional, without risking parallel runs.
                if prepare_only
                    && error
                        .downcast_ref::<crate::client::ApiError>()
                        .is_some_and(|e| e.status == 404)
                {
                    return Ok(());
                }
                if once {
                    return Err(error);
                }
                eprintln!("Connection interrupted. Retrying in {retry_delay}s: {error}");
                retry_delay = (retry_delay * 2).min(60);
            }
        }
        if once {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(retry_delay)).await;
    }
}

fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        && output.status.success()
        && let Ok(name) = String::from_utf8(output.stdout)
        && !name.trim().is_empty()
    {
        return name.trim().into();
    }
    if let Ok(output) = std::process::Command::new("hostname").output()
        && output.status.success()
        && let Ok(name) = String::from_utf8(output.stdout)
        && !name.trim().is_empty()
    {
        return name.trim().trim_end_matches(".local").into();
    }
    "This computer".into()
}

#[cfg(test)]
mod reply_tests {
    use super::*;

    #[test]
    fn claude_provider_error_is_not_mistaken_for_successful_subtype() {
        let error = decode_claude_reply(br#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: 422 Budget has been exceeded!"}"#).unwrap_err();
        assert!(error.to_string().contains("Budget has been exceeded"));
        assert!(decode_claude_reply(b"").is_err());
        let reply = decode_claude_reply(
            br#"{"is_error":false,"structured_output":{"text":"Done","files":[]}}"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&reply).unwrap(),
            json!({"text":"Done","files":[]})
        );
    }
}
