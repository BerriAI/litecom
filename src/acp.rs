//! ACP v1 client. Native tools stay in the agent; new permissions go to its owner.
use crate::{
    client::Client,
    config,
    model::{Decision, Status, ToolPermission, ToolPermissionInput, WorkItem},
    runner::{AgentReply, ProcessGroup, RunnerConfig},
};
use agent_client_protocol::{
    Agent, ByteStreams, ConnectionTo, Error as AcpError,
    schema::{ProtocolVersion, v1::*},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

#[derive(Default)]
struct Session {
    id: Option<SessionId>,
    connection: Option<ConnectionTo<Agent>>,
    text: String,
    tools: BTreeMap<String, Value>,
}
type Shared = Arc<Mutex<Session>>;

fn merge_scope(session: &mut Session, id: String, update: Value) -> Value {
    let scope = session.tools.entry(id).or_insert_with(|| json!({}));
    // Status and output are progress, not changes to the proposed operation.
    for field in ["title", "name", "kind", "rawInput", "content", "locations"] {
        if let Some(value) = update.get(field).filter(|value| !value.is_null()) {
            scope[field] = value.clone();
        }
    }
    scope.clone()
}

async fn permission(
    client: Client,
    work: WorkItem,
    shared: Shared,
    request: RequestPermissionRequest,
) -> Result<RequestPermissionOutcome> {
    let choices = request
        .options
        .iter()
        .filter(|o| o.kind == PermissionOptionKind::AllowOnce)
        .collect::<Vec<_>>();
    // A generic approval button cannot choose between scopes or grant persistence.
    if choices.len() != 1 {
        return Ok(RequestPermissionOutcome::Cancelled);
    }
    let scope = {
        let mut session = shared.lock().expect("ACP session lock");
        if session.id.as_ref() != Some(&request.session_id) {
            return Ok(RequestPermissionOutcome::Cancelled);
        }
        merge_scope(
            &mut session,
            request.tool_call.tool_call_id.to_string(),
            serde_json::to_value(&request.tool_call)?,
        )
    };
    let has_detail = scope
        .get("rawInput")
        .is_some_and(|v| !v.is_null() && v != &json!({}))
        || ["content", "locations"]
            .iter()
            .any(|key| scope[*key].as_array().is_some_and(|a| !a.is_empty()));
    if !has_detail {
        return Ok(RequestPermissionOutcome::Cancelled);
    }
    let title = scope["title"]
        .as_str()
        .filter(|title| !title.trim().is_empty() && title.len() <= 200)
        .unwrap_or("Approve agent operation");
    let input = ToolPermissionInput {
        id: crate::crypto::id(),
        title: title.into(),
        details: json!({"protocol":"ACP v1", "operation":scope, "permission":choices[0], "request":request}),
    };
    let path = format!("/api/tasks/{}/permissions", work.task_id);
    let mut pending: ToolPermission = client
        .post(&path, json!({"lease":work.lease,"permission":input}))
        .await?;
    loop {
        if let Some(decision) = pending.decision {
            // An agent must ask again if it changes the scope while waiting.
            let unchanged = shared
                .lock()
                .expect("ACP session lock")
                .tools
                .get(&request.tool_call.tool_call_id.to_string())
                == Some(&scope);
            if !unchanged {
                bail!("The ACP operation changed while awaiting approval. It was not authorized.");
            }
            let choice = if decision == Decision::Approve {
                Some(choices[0])
            } else {
                request
                    .options
                    .iter()
                    .find(|o| o.kind == PermissionOptionKind::RejectOnce)
            };
            return Ok(choice.map_or(RequestPermissionOutcome::Cancelled, |o| {
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                    o.option_id.clone(),
                ))
            }));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        pending = client
            .post(
                &format!("{path}/{}/status", pending.id),
                json!({"lease":work.lease}),
            )
            .await?;
    }
}

pub(crate) async fn reply(
    client: &Client,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
) -> Result<AgentReply> {
    let stderr = dir.join("agent-error.log");
    config::write_private(&stderr, b"")?;
    let mut command = tokio::process::Command::new(&settings.executable);
    command
        .args(&settings.arguments)
        .current_dir(&settings.workspace)
        .env("PWD", &settings.workspace)
        .env_remove("LITECOM_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&stderr)?)
        .kill_on_drop(true);
    if settings.agent == crate::runner::NativeAgent::OpenClaw {
        command
            .env("OPENCLAW_HIDE_BANNER", "1")
            .env("OPENCLAW_SUPPRESS_NOTES", "1");
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("Could not start the selected ACP agent.")?;
    let _group = ProcessGroup(child.id());
    let transport = ByteStreams::new(
        child
            .stdin
            .take()
            .context("Missing ACP input")?
            .compat_write(),
        child.stdout.take().context("Missing ACP output")?.compat(),
    );
    converse(transport, client, settings, work, dir).await
}

pub(crate) async fn converse(
    transport: impl agent_client_protocol::ConnectTo<agent_client_protocol::Client> + 'static,
    client: &Client,
    settings: &RunnerConfig,
    work: &WorkItem,
    dir: &Path,
) -> Result<AgentReply> {
    let shared = Arc::new(Mutex::new(Session::default()));
    let notifications = shared.clone();
    let permissions = shared.clone();
    let session_state = shared.clone();
    let worker = client.clone();
    let task = work.clone();
    let cwd = settings.workspace.clone();
    let mcp = settings.mcp.clone();
    let prompt = format!(
        "{}\n\nYour final response must be only JSON with this shape: {{\"text\":\"your full reply\",\"files\":[\"/absolute/path/to/file\"]}}. Use an empty files array if no attachment is needed. Do not put commentary after the JSON.",
        crate::runner::prompt(work)
    );
    let connection = agent_client_protocol::Client
        .builder()
        .name("Litecom")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let mut session = notifications.lock().expect("ACP session lock");
                if session.id.as_ref() != Some(&notification.session_id) {
                    return Ok(());
                }
                match notification.update {
                    SessionUpdate::AgentMessageChunk(chunk) => {
                        if let ContentBlock::Text(text) = chunk.content {
                            session.text.push_str(&text.text);
                        }
                    }
                    SessionUpdate::ToolCall(call) => {
                        merge_scope(
                            &mut session,
                            call.tool_call_id.to_string(),
                            serde_json::to_value(call).map_err(AcpError::into_internal_error)?,
                        );
                    }
                    SessionUpdate::ToolCallUpdate(call) => {
                        merge_scope(
                            &mut session,
                            call.tool_call_id.to_string(),
                            serde_json::to_value(call).map_err(AcpError::into_internal_error)?,
                        );
                    }
                    _ => {}
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, cx| {
                let worker = worker.clone();
                let task = task.clone();
                let state = permissions.clone();
                cx.spawn(async move {
                    match permission(worker, task, state, request).await {
                        Ok(outcome) => responder.respond(RequestPermissionResponse::new(outcome)),
                        Err(error) => {
                            responder.respond(RequestPermissionResponse::new(
                                RequestPermissionOutcome::Cancelled,
                            ))?;
                            Err(AcpError::internal_error().data(error.to_string()))
                        }
                    }
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |cx| {
            // No optional filesystem/terminal capability is advertised: tools
            // execute in the native agent under its existing configuration.
            let initialize = InitializeRequest::new(ProtocolVersion::V1)
                .client_info(Implementation::new("litecom", env!("CARGO_PKG_VERSION")));
            let initialized = tokio::time::timeout(
                Duration::from_secs(30),
                cx.send_request(initialize).block_task(),
            )
            .await
            .map_err(AcpError::into_internal_error)??;
            if initialized.protocol_version != ProtocolVersion::V1 {
                return Err(AcpError::internal_error().data("This agent did not negotiate ACP v1."));
            }
            let mut new_session = NewSessionRequest::new(cwd);
            if let Some(mcp) = mcp {
                new_session.mcp_servers.push(McpServer::Stdio(
                    McpServerStdio::new("litecom", mcp.executable).args(vec![
                        "--home".into(),
                        mcp.home,
                        "--connection".into(),
                        mcp.connection,
                        "mcp".into(),
                    ]),
                ));
            }
            let created = tokio::time::timeout(
                Duration::from_secs(30),
                cx.send_request(new_session).block_task(),
            )
            .await
            .map_err(AcpError::into_internal_error)??;
            {
                let mut session = session_state.lock().expect("ACP session lock");
                session.id = Some(created.session_id.clone());
                session.connection = Some(cx.clone());
            }
            let response = cx
                .send_request(PromptRequest::new(
                    created.session_id,
                    vec![ContentBlock::Text(TextContent::new(prompt))],
                ))
                .block_task()
                .await?;
            if response.stop_reason != StopReason::EndTurn {
                return Err(AcpError::internal_error().data(format!(
                    "ACP agent stopped with {:?}; the result was not staged.",
                    response.stop_reason
                )));
            }
            Ok(())
        });
    tokio::pin!(connection);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let result = loop {
        tokio::select! {
            result = &mut connection => break result.map_err(anyhow::Error::from),
            _ = heartbeat.tick() => {
                let state: Result<Status> = client.post(&format!("/api/tasks/{}/heartbeat",work.task_id),json!({"lease":work.lease})).await;
                if !matches!(state, Ok(Status::Executing)) {
                    {
                        let session = shared.lock().expect("ACP session lock");
                        if let (Some(id),Some(cx)) = (&session.id,&session.connection) {
                            let _ = cx.send_notification(CancelNotification::new(id.clone()));
                        }
                    }
                    // Let the protocol cancellation flush, then stop descendants.
                    let _ = tokio::time::timeout(Duration::from_secs(2), &mut connection).await;
                    break Err(anyhow::anyhow!("The request stopped or its lease could not be renewed. Check for effects that already happened."));
                }
            }
        }
    };
    let output = shared.lock().expect("ACP session lock").text.clone();
    config::write_private(&dir.join("agent-reply.txt"), output.as_bytes())?;
    result.context("The ACP run did not complete. Its private diagnostics were kept.")?;
    parse_reply(&output)
}

fn parse_reply(output: &str) -> Result<AgentReply> {
    // ACP streams can include a short progress message before the final JSON.
    // Accept only a complete reply at the end, never a partial embedded example.
    let output = output
        .trim()
        .strip_suffix("```")
        .unwrap_or(output.trim())
        .trim();
    for (offset, _) in output.match_indices('{') {
        if let Ok(reply) = serde_json::from_str::<AgentReply>(&output[offset..]) {
            if reply.text.len() > 2_000_000 || reply.files.len() > 100 {
                bail!("The agent returned more than Litecom can transfer in one request.");
            }
            return Ok(reply);
        }
    }
    bail!("The agent did not return the requested reply and attachment list. No result was shared.")
}
