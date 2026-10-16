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
    let stderr = dir.join("agent-error.log");
    config::write_private(&stderr, b"")?;
    let mut command = tokio::process::Command::new(&settings.executable);
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(&settings.workspace)
        .env("PWD", &settings.workspace)
        .env_remove("LITECOM_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(stderr)?)
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("Could not start Codex app-server. Update Codex, then retry.")?;
    let _group = ProcessGroup(child.id());
    converse(
        child.stdout.take().context("Missing Codex output")?,
        child.stdin.take().context("Missing Codex input")?,
        settings,
        work,
        dir,
        renewal,
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
        if !self.owns_request(method, params) {
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
        if self.unanswered_interaction {
            reply.text.push_str("\n\nLitecom could not answer an interactive prompt; this does not mean the owner declined. The owner needs to review that step in their agent before retrying.");
        }
        Ok(reply)
    }
}

pub(crate) async fn converse(
    reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
    renewal: &mut WorkLease<'_>,
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
    let mut session = Session::default();
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line?.context("Codex disconnected before returning a result. Check private run diagnostics.")?,
            _ = heartbeat.tick() => { authorized(renewal).await?; continue; }
        };
        let message: Value =
            serde_json::from_str(&line).context("Codex returned invalid protocol data.")?;
        log.write_all(line.as_bytes()).await?;
        log.write_all(b"\n").await?;
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
                    send(
                        &mut writer,
                        json!({"id":2,"method":"thread/start","params":{
                            "cwd":settings.workspace,"approvalPolicy":"on-request",
                            "approvalsReviewer":"user","sandbox":"workspace-write"
                        }}),
                    )
                    .await?;
                }
                2 => {
                    let id = message["result"]["thread"]["id"]
                        .as_str()
                        .context("Codex did not create a session.")?;
                    session.thread = Some(id.into());
                    config::write_private(&dir.join("session-id"), id.as_bytes())?;
                    authorized(renewal).await?;
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
