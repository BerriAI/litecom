//! One Codex app-server session per approved Litecom run. Tool grants never persist.
use crate::{
    client::WorkLease,
    config,
    model::{Status, WorkItem},
    runner::{AgentReply, ProcessGroup, RunnerConfig},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

pub(crate) async fn reply(
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
) -> Result<AgentReply> {
    launch(settings, work, dir, renewal, None).await
}

pub(crate) async fn reply_read_only(
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
    session: &Path,
) -> Result<AgentReply> {
    launch(settings, work, dir, renewal, Some(session)).await
}

async fn launch(
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
    read_session: Option<&Path>,
) -> Result<AgentReply> {
    let isolated = read_session.map(|p| p.parent().expect("session parent").join("agent"));
    if let Some(path) = &isolated {
        config::private_dir(path)?;
    }
    let cwd = isolated.as_deref().unwrap_or(&settings.workspace);
    let stderr = dir.join("agent-error.log");
    config::write_private(&stderr, b"")?;
    let mut command = tokio::process::Command::new(&settings.executable);
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(cwd)
        .env("PWD", cwd)
        .env_remove("LITECOM_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(stderr)?)
        .kill_on_drop(true);
    if read_session.is_some() {
        crate::read_only::codex_startup(&mut command);
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("Could not start Codex app-server. Update Codex, then retry.")?;
    let _group = ProcessGroup(child.id());
    converse_inner(
        child.stdout.take().context("Missing Codex output")?,
        child.stdin.take().context("Missing Codex input")?,
        settings,
        work,
        dir,
        renewal,
        read_session,
    )
    .await
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), message: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&message)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn authorized(renewal: &mut WorkLease<'_>) -> Result<()> {
    if renewal.renew().await? != Status::Executing {
        bail!(
            "The request is no longer approved to run. No further tool permissions were granted."
        );
    }
    Ok(())
}

#[derive(Default)]
struct Session {
    thread: Option<String>,
    turn: Option<String>,
    mcp_items: BTreeSet<String>,
    output: Option<String>,
    unanswered_interaction: bool,
    read_only: bool,
}
impl Session {
    fn belongs(&self, params: &Value) -> bool {
        self.thread.is_some()
            && self.turn.is_some()
            && params["threadId"].as_str() == self.thread.as_deref()
            && params["turnId"].as_str() == self.turn.as_deref()
    }

    fn owns_request(&self, method: &str, params: &Value) -> bool {
        let current_mcp = method == "mcpServer/elicitation/request"
            && self.thread.is_some()
            && self.turn.is_some()
            && params["threadId"].as_str() == self.thread.as_deref()
            && params["turnId"].is_null();
        self.belongs(params) || current_mcp
    }

    fn approval(&self, method: &str, params: &Value) -> Option<Value> {
        if self.read_only || !self.owns_request(method, params) {
            return None;
        }
        match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                if let Some(choices) = params["availableDecisions"].as_array()
                    && !choices.iter().any(|v| v == "accept")
                {
                    return None;
                }
                Some(json!({"decision":"accept"}))
            }
            "item/permissions/requestApproval" if params["permissions"].is_object() => {
                Some(json!({"permissions":params["permissions"],"scope":"turn"}))
            }
            // Codex marks tool confirmations separately from forms asking for
            // information, secrets or sign-in. Never fabricate answers to those.
            "mcpServer/elicitation/request"
                if matches!(
                    params["mode"].as_str(),
                    Some("form" | "openai/form" | "openaiForm")
                ) && params["_meta"]["codex_approval_kind"] == "mcp_tool_call"
                    && params["requestedSchema"]["properties"]
                        .as_object()
                        .is_some_and(|p| p.is_empty()) =>
            {
                Some(json!({"action":"accept","content":{}}))
            }
            // Older app-server versions represent connector confirmations as
            // requestUserInput. Match the active MCP item, not arbitrary prose.
            "item/tool/requestUserInput" => {
                let item = params["itemId"].as_str()?;
                if !self.mcp_items.contains(item) {
                    return None;
                }
                let questions = params["questions"].as_array()?;
                if questions.len() != 1 {
                    return None;
                }
                let q = &questions[0];
                if q["header"] != "Approve app tool call?"
                    || q["isSecret"] == true
                    || q["isOther"] == true
                {
                    return None;
                }
                let label = q["options"].as_array()?.iter().find_map(|o| {
                    o["label"]
                        .as_str()
                        .filter(|label| matches!(*label, "Accept" | "Allow"))
                })?;
                let id = q["id"].as_str()?;
                Some(json!({"answers":{id:{"answers":[label]}}}))
            }
            _ => None,
        }
    }

    fn unanswered(&mut self, method: &str, params: &Value) -> Option<Value> {
        let result = match method {
            "mcpServer/elicitation/request" => json!({"action":"cancel","content":null}),
            "item/tool/requestUserInput" => json!({"answers":{}}),
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                json!({"decision":"cancel"})
            }
            "item/permissions/requestApproval" => json!({"permissions":{},"scope":"turn"}),
            _ => return None,
        };
        // A client cancellation is not a human denial. Report it without
        // copying private prompt details into the requester-visible reply.
        self.unanswered_interaction |= self.owns_request(method, params);
        Some(result)
    }

    fn final_reply(&self, output: &str) -> Result<AgentReply> {
        let mut reply: AgentReply = serde_json::from_str(output)
            .context("Codex's reply did not match the expected format.")?;
        if self.unanswered_interaction && self.read_only {
            reply.text.push_str("\n\nA tool operation was blocked by read-only access. Changes need a new action request.");
        } else if self.unanswered_interaction {
            reply.text.push_str("\n\nLitecom could not answer an interactive prompt; this does not mean the owner declined. The owner needs to review that step in their agent before retrying.");
        }
        Ok(reply)
    }
}

#[cfg(test)]
pub(crate) async fn converse(
    reader: impl AsyncRead + Unpin,
    writer: impl AsyncWrite + Unpin,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
) -> Result<AgentReply> {
    converse_inner(reader, writer, settings, work, dir, renewal, None).await
}

#[allow(clippy::too_many_arguments)]
async fn converse_inner(
    reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
    read_session: Option<&Path>,
) -> Result<AgentReply> {
    authorized(renewal).await?;
    let log = dir.join("agent-events.jsonl");
    config::write_private(&log, b"")?;
    let mut log = tokio::fs::OpenOptions::new().append(true).open(log).await?;
    send(
        &mut writer,
        json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"litecom","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true,"explicitGatewayOauth":true}
        }}),
    )
    .await?;
    let mut lines = BufReader::new(reader).lines();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut session = Session {
        read_only: read_session.is_some(),
        ..Default::default()
    };
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line?.context("Codex disconnected before returning a result. Check private run diagnostics.")?,
            _ = heartbeat.tick() => { authorized(renewal).await?; continue; }
        };
        let message: Value =
            serde_json::from_str(&line).context("Codex returned invalid protocol data.")?;
        // Effective configuration can contain provider credentials. Never put
        // config/read output in run logs or return it to the model.
        if message["id"] != 4 {
            log.write_all(line.as_bytes()).await?;
            log.write_all(b"\n").await?;
        }
        if let Some(method) = message["method"].as_str() {
            let params = &message["params"];
            if message.get("id").is_some() {
                // Verify current authority for every grant, including after a
                // disconnect/redeploy. The initial approval alone is not a lease.
                let result = if let Some(result) = session.approval(method, params) {
                    authorized(renewal).await?;
                    json!({"id":message["id"],"result":result})
                } else {
                    match session.unanswered(method, params) {
                        Some(result) => json!({"id":message["id"],"result":result}),
                        None => {
                            json!({"id":message["id"],"error":{"code":-32601,"message":"Litecom cannot answer this interactive request during an unattended run."}})
                        }
                    }
                };
                send(&mut writer, result).await?;
            } else if params["threadId"].as_str() == session.thread.as_deref()
                && session.thread.is_some()
            {
                if method == "turn/started" && session.turn.is_none() {
                    session.turn = params["turn"]["id"].as_str().map(str::to_owned);
                } else if session.belongs(params) {
                    if method == "item/started" && params["item"]["type"] == "mcpToolCall" {
                        if let Some(id) = params["item"]["id"].as_str() {
                            session.mcp_items.insert(id.into());
                        }
                    } else if method == "item/completed" {
                        let item = &params["item"];
                        if item["type"] == "agentMessage"
                            && let Some(text) = item["text"].as_str()
                        {
                            session.output = Some(text.into());
                        }
                        if let Some(id) = item["id"].as_str() {
                            session.mcp_items.remove(id);
                        }
                    }
                }
                // turn/completed carries turn.id rather than turnId.
                if method == "turn/completed"
                    && params["turn"]["id"].as_str() == session.turn.as_deref()
                {
                    authorized(renewal).await?;
                    if params["turn"]["status"] != "completed" {
                        bail!("Codex did not complete: {}", params["turn"]["error"]);
                    }
                    let output = session
                        .output
                        .as_deref()
                        .context("Codex did not return a final reply.")?;
                    let reply = session.final_reply(output)?;
                    config::write_private(
                        &dir.join("reply.json"),
                        &serde_json::to_vec(&json!({"text":reply.text,"files":reply.files}))?,
                    )?;
                    return Ok(reply);
                }
            }
        } else if let Some(id) = message["id"].as_u64() {
            if let Some(error) = message.get("error") {
                bail!("Codex could not start the approved run: {error}");
            }
            match id {
                1 => {
                    send(&mut writer, json!({"method":"initialized"})).await?;
                    if read_session.is_some() {
                        send(
                            &mut writer,
                            json!({"id":4,"method":"config/read","params":{"includeLayers":false}}),
                        )
                        .await?;
                        continue;
                    }
                    send(
                        &mut writer,
                        json!({"id":2,"method":"thread/start","params":{
                            "cwd":settings.workspace,"approvalPolicy":"on-request",
                            "approvalsReviewer":"user","sandbox":"workspace-write"
                        }}),
                    )
                    .await?;
                }
                4 => {
                    let path = read_session.context("Unexpected config response")?;
                    let overrides =
                        crate::read_only::codex_config(&message["result"]["config"], path)?;
                    send(&mut writer,json!({"id":2,"method":"thread/start","params":{
                        "cwd":path.parent().expect("session parent").join("agent"),"approvalPolicy":"never","sandbox":"read-only","ephemeral":true,"config":overrides
                    }})).await?;
                }
                5 => {
                    let servers = message["result"]["data"]
                        .as_array()
                        .context("Could not verify the read-only tool list")?;
                    if !message["result"]["nextCursor"].is_null()
                        || servers.iter().any(|s| {
                            s["name"] != "litecom_read"
                                && (s["runtimeStatus"] != "disabled"
                                    || !s["tools"].as_object().is_some_and(|t| t.is_empty()))
                        })
                    {
                        bail!(
                            "Unexpected external tools were available; read-only startup was stopped."
                        );
                    }
                    let safe = servers
                        .iter()
                        .find(|s| s["name"] == "litecom_read" && s["runtimeStatus"] == "connected")
                        .context("Read tools did not connect")?;
                    let tools = safe["tools"]
                        .as_object()
                        .context("Read tools did not start")?;
                    if tools.len() != 3
                        || !tools.values().all(|tool| {
                            matches!(
                                tool["name"].as_str(),
                                Some("read_file" | "github_read" | "slack_read")
                            )
                        })
                    {
                        bail!("The read-only tool catalog could not be verified.");
                    }
                    authorized(renewal).await?;
                    send(&mut writer,json!({"id":3,"method":"turn/start","params":{
                        "threadId":session.thread,"input":[{"type":"text","text":crate::read_only::prompt(work)}],"outputSchema":crate::runner::reply_schema()
                    }})).await?;
                }
                2 => {
                    if read_session.is_some()
                        && (message["result"]["sandbox"]["type"] != "readOnly"
                            || message["result"]["approvalPolicy"] != "never")
                    {
                        bail!(
                            "Codex did not apply read-only permissions. This request was not run."
                        );
                    }
                    let id = message["result"]["thread"]["id"]
                        .as_str()
                        .context("Codex did not create a session.")?;
                    session.thread = Some(id.into());
                    config::write_private(&dir.join("session-id"), id.as_bytes())?;
                    authorized(renewal).await?;
                    if read_session.is_some() {
                        send(&mut writer,json!({"id":5,"method":"mcpServerStatus/list","params":{"threadId":id,"limit":100}})).await?;
                        continue;
                    }
                    send(&mut writer, json!({"id":3,"method":"turn/start","params":{
                        "threadId":id,"input":[{"type":"text","text":crate::runner::prompt(work)}],
                        "outputSchema":crate::runner::reply_schema()
                    }})).await?;
                }
                3 => {
                    let id = message["result"]["turn"]["id"]
                        .as_str()
                        .context("Codex did not start a turn.")?;
                    if session.turn.as_deref().is_some_and(|active| active != id) {
                        bail!("Codex returned an unexpected turn.");
                    }
                    session.turn = Some(id.into());
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{"text":"No text was sent.","files":[]}"#;

    fn session() -> Session {
        Session {
            thread: Some("thread-1".into()),
            turn: Some("turn-1".into()),
            mcp_items: BTreeSet::from(["mcp-1".into()]),
            ..Default::default()
        }
    }

    #[test]
    fn read_only_sessions_never_grant_tool_or_sandbox_permissions() {
        let mut session = session();
        session.read_only = true;
        for method in [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
            "item/permissions/requestApproval",
            "mcpServer/elicitation/request",
            "item/tool/requestUserInput",
        ] {
            let params = json!({"threadId":"thread-1","turnId":"turn-1","itemId":"mcp-1","permissions":{"network":{"enabled":true}},"mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call"},"requestedSchema":{"properties":{}}});
            assert!(session.approval(method, &params).is_none());
            assert!(session.unanswered(method, &params).is_some());
        }
        let reply = session.final_reply(REPLY).unwrap();
        assert!(reply.text.contains("blocked by read-only access"));
    }

    #[test]
    fn app_access_sign_in_and_questions_remain_unanswered_and_are_explained() {
        // Synthetic prompts exercise the transport without accessing an app.
        for (method, extra, expected) in [
            (
                "mcpServer/elicitation/request",
                json!({"mode":"form","message":"private-fixture: grant app access?","requestedSchema":{"properties":{"allow":{"type":"boolean"}}}}),
                json!({"action":"cancel","content":null}),
            ),
            (
                "mcpServer/elicitation/request",
                json!({"mode":"url","url":"https://example.invalid/private-fixture","elicitationId":"login"}),
                json!({"action":"cancel","content":null}),
            ),
            (
                "mcpServer/elicitation/request",
                json!({"turnId":null,"mode":"form","requestedSchema":{"properties":{"password":{"type":"string"}}}}),
                json!({"action":"cancel","content":null}),
            ),
            (
                "item/tool/requestUserInput",
                json!({"itemId":"mcp-1","questions":[{"id":"q","header":"Approve Messages access?","question":"private-fixture","options":[{"label":"Allow"},{"label":"Cancel"}]}]}),
                json!({"answers":{}}),
            ),
        ] {
            let mut session = session();
            let mut params = json!({"threadId":"thread-1","turnId":"turn-1"});
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(session.approval(method, &params).is_none());
            assert_eq!(session.unanswered(method, &params), Some(expected));
            let reply = session.final_reply(REPLY).unwrap();
            assert!(reply.text.starts_with("No text was sent."));
            assert!(
                reply
                    .text
                    .contains("Litecom could not answer an interactive prompt")
            );
            assert!(reply.text.contains("does not mean the owner declined"));
            assert!(!reply.text.contains("private-fixture"));
            assert!(reply.files.is_empty());
        }
    }

    #[test]
    fn unrelated_requests_do_not_add_a_notice_to_this_reply() {
        let mut session = session();
        for (thread, turn) in [("other", "turn-1"), ("thread-1", "other")] {
            let params = json!({"threadId":thread,"turnId":turn,"mode":"form"});
            assert!(
                session
                    .approval("mcpServer/elicitation/request", &params)
                    .is_none()
            );
            assert_eq!(
                session.unanswered("mcpServer/elicitation/request", &params),
                Some(json!({"action":"cancel","content":null}))
            );
        }
        assert!(
            session
                .unanswered(
                    "unknown/method",
                    &json!({"threadId":"thread-1","turnId":"turn-1"})
                )
                .is_none()
        );
        assert_eq!(
            session.final_reply(REPLY).unwrap().text,
            "No text was sent."
        );
    }

    #[test]
    fn unanswered_notice_is_once_only_and_preserves_attachments() {
        let mut session = session();
        for _ in 0..2 {
            session.unanswered(
                "item/tool/requestUserInput",
                &json!({"threadId":"thread-1","turnId":"turn-1"}),
            );
        }
        let reply = session
            .final_reply(r#"{"text":"Partial result.","files":["/tmp/requested-file.txt"]}"#)
            .unwrap();
        assert_eq!(reply.text.matches("Litecom could not answer").count(), 1);
        assert_eq!(
            reply.files,
            vec![std::path::PathBuf::from("/tmp/requested-file.txt")]
        );
        assert!(session.final_reply("not JSON").is_err());
    }
}
