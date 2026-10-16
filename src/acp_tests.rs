use super::*;
use crate::{
    api::AppState,
    client::Client,
    config::ClientConfig,
    runner::{NativeAgent, RunnerConfig},
};
use agent_client_protocol::{
    ByteStreams,
    schema::{ProtocolVersion, v1::*},
};
use serde_json::json;
use std::time::Duration;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

struct Harness {
    state: AppState,
    client: Client,
    owner: Principal,
    requester: Principal,
    worker: Principal,
    settings: RunnerConfig,
    work: WorkItem,
    temp: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Harness {
    async fn new() -> Self {
        Self::new_with(NativeAgent::Acp, None, "Please send the document").await
    }
    async fn new_with(
        agent: NativeAgent,
        executable: Option<std::path::PathBuf>,
        request: &str,
    ) -> Self {
        let f = Fixture::new();
        let temp = tempfile::tempdir().unwrap();
        let settings = RunnerConfig {
            agent,
            executable: executable.unwrap_or_else(|| std::env::current_exe().unwrap()),
            workspace: temp.path().into(),
            arguments: vec![],
            mcp: agent.is_acp().then(|| McpConnection {
                executable: std::env::current_exe().unwrap().to_string_lossy().into(),
                home: temp.path().to_string_lossy().into(),
                connection: "test".into(),
            }),
        };
        let device = f.store.begin_device("ACP test").unwrap();
        f.store
            .authorize_device(&f.b, &device.user_code, &f.agent)
            .unwrap();
        let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
        let worker = f
            .store
            .authenticate(&token.token, crate::model::Role::Agent)
            .unwrap();
        let task = f
            .store
            .create_task(
                &f.a,
                NewTask {
                    ask_only: false,
                    owner_id: f.b.user_id.clone(),
                    agent_id: Some(f.agent.clone()),
                    request: request.into(),
                    idempotency_key: "protocol-test".into(),
                },
            )
            .unwrap();
        let prepare = f.store.claim(&worker, &f.agent).unwrap().unwrap();
        let prepared = f
            .store
            .prepare(
                &worker,
                &task.id,
                Some(&prepare.lease),
                DraftInput {
                    action: Some(settings.action(&task.request)),
                    ..Default::default()
                },
            )
            .unwrap();
        f.approve(&prepared);
        let work = f.store.claim(&worker, &f.agent).unwrap().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = AppState::new(
            f.store,
            origin.clone(),
            crate::providers::Providers::development().unwrap(),
        );
        let router = crate::api::router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = Client::new(ClientConfig {
            relay: origin,
            token: token.token,
            user_id: f.b.user_id.clone(),
            agent_id: f.agent,
            workspace: None,
            runner: None,
            approval_channel: None,
        })
        .unwrap();
        Self {
            state,
            client,
            owner: f.b,
            requester: f.a,
            worker,
            settings,
            work,
            temp,
            server,
        }
    }
}

#[derive(Clone, Copy)]
enum Case {
    Once,
    PersistentOnly,
    MultipleChoices,
    WrongSession,
}

fn mock_agent(
    stream: tokio::io::DuplexStream,
    folder: std::path::PathBuf,
    case: Case,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<RequestPermissionOutcome>,
) {
    let (send, receive) = tokio::sync::mpsc::unbounded_channel();
    let (read, write) = tokio::io::split(stream);
    let handle = tokio::spawn(async move {
        agent_client_protocol::Agent
            .builder()
            .on_receive_request(
                async |request: InitializeRequest, responder, _| {
                    assert_eq!(request.protocol_version, ProtocolVersion::V1);
                    assert!(!request.client_capabilities.terminal);
                    assert!(!request.client_capabilities.fs.read_text_file);
                    responder.respond(InitializeResponse::new(ProtocolVersion::V1))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: NewSessionRequest, responder, _| {
                    assert_eq!(request.cwd, folder);
                    assert_eq!(request.mcp_servers.len(), 1);
                    let request = serde_json::to_value(request).unwrap();
                    assert_eq!(request["mcpServers"][0]["name"], "litecom");
                    assert_eq!(request["mcpServers"][0]["args"][3], "test");
                    assert!(request.get("model").is_none() && request.get("max_tokens").is_none());
                    responder.respond(NewSessionResponse::new("fixture-session"))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: PromptRequest, responder, cx| {
                    let send = send.clone();
                    cx.clone().spawn(async move {
                        assert_eq!(request.session_id.to_string(), "fixture-session");
                        let scope = ToolCallUpdateFields::new()
                            .title("Write the requested note")
                            .raw_input(json!({"path":"note.txt","content":"approved content"}));
                        // The permission request can omit fields sent in earlier updates.
                        cx.send_notification(SessionNotification::new(
                            "fixture-session",
                            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("write-1", scope)),
                        ))?;
                        cx.send_notification(SessionNotification::new(
                            "fixture-session",
                            SessionUpdate::AgentThoughtChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(
                                    "private reasoning must not be in result",
                                )),
                            )),
                        ))?;
                        let mut options = vec![PermissionOption::new(
                            "always",
                            "Always allow",
                            PermissionOptionKind::AllowAlways,
                        )];
                        if !matches!(case, Case::PersistentOnly) {
                            options.push(PermissionOption::new(
                                "once",
                                "Allow this write",
                                PermissionOptionKind::AllowOnce,
                            ));
                        }
                        if matches!(case, Case::MultipleChoices) {
                            options.push(PermissionOption::new(
                                "other",
                                "Choose a different operation",
                                PermissionOptionKind::AllowOnce,
                            ));
                        }
                        options.push(PermissionOption::new(
                            "no",
                            "Decline",
                            PermissionOptionKind::RejectOnce,
                        ));
                        let session = if matches!(case, Case::WrongSession) {
                            "other-session"
                        } else {
                            "fixture-session"
                        };
                        let outcome = cx
                            .send_request(RequestPermissionRequest::new(
                                session,
                                ToolCallUpdate::new("write-1", ToolCallUpdateFields::new()),
                                options,
                            ))
                            .block_task()
                            .await?;
                        send.send(outcome.outcome.clone()).unwrap();
                        let reply =
                            json!({"text":"Here is the completed reply.","files":[]}).to_string();
                        for text in ["I will prepare the result.\n", &reply] {
                            cx.send_notification(SessionNotification::new(
                                "fixture-session",
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new(text)),
                                )),
                            ))?;
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(ByteStreams::new(write.compat_write(), read.compat()))
            .await
            .ok();
    });
    (handle, receive)
}

#[tokio::test]
async fn cursor_authenticates_before_opening_a_session_and_stops_on_login_failure() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    for (advertised, authenticated) in [(true, true), (true, false), (false, false)] {
        let mut h = Harness::new().await;
        h.settings.agent = NativeAgent::Cursor;
        h.settings.mcp = None;
        let auth_seen = Arc::new(AtomicBool::new(false));
        let session_seen = Arc::new(AtomicBool::new(false));
        let auth = auth_seen.clone();
        let auth_before_session = auth_seen.clone();
        let session = session_seen.clone();
        let (client_io, agent_io) = tokio::io::duplex(65536);
        let (read, write) = tokio::io::split(agent_io);
        let peer = tokio::spawn(async move {
            agent_client_protocol::Agent
                .builder()
                .on_receive_request(
                    async move |_: InitializeRequest, responder, _| {
                        let methods = if advertised {
                            vec![AuthMethod::Agent(AuthMethodAgent::new(
                                "cursor_login",
                                "Cursor Login",
                            ))]
                        } else {
                            vec![]
                        };
                        responder.respond(
                            InitializeResponse::new(ProtocolVersion::V1).auth_methods(methods),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: AuthenticateRequest, responder, _| {
                        assert_eq!(request.method_id.to_string(), "cursor_login");
                        assert!(!auth.swap(true, Ordering::SeqCst), "authenticate once");
                        if authenticated {
                            responder.respond(AuthenticateResponse::new())
                        } else {
                            responder.respond_with_error(
                                agent_client_protocol::Error::internal_error()
                                    .data("expired login"),
                            )
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: NewSessionRequest, responder, _| {
                        assert!(authenticated && auth_before_session.load(Ordering::SeqCst));
                        assert!(request.mcp_servers.is_empty());
                        session.store(true, Ordering::SeqCst);
                        responder.respond(NewSessionResponse::new("cursor-test"))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |request: PromptRequest, responder, cx| {
                        cx.send_notification(SessionNotification::new(
                            request.session_id,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(
                                    r#"{"text":"Authenticated reply","files":[]}"#,
                                )),
                            )),
                        ))?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(ByteStreams::new(write.compat_write(), read.compat()))
                .await
                .ok();
        });
        let (read, write) = tokio::io::split(client_io);
        let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            crate::acp::converse(
                ByteStreams::new(write.compat_write(), read.compat()),
                &h.client,
                &h.settings,
                &h.work,
                h.temp.path(),
                &mut renewal,
            ),
        )
        .await
        .unwrap();
        assert_eq!(auth_seen.load(Ordering::SeqCst), advertised);
        assert_eq!(session_seen.load(Ordering::SeqCst), authenticated);
        if authenticated {
            assert_eq!(result.unwrap().text, "Authenticated reply");
        } else {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains(if advertised {
                    "agent login"
                } else {
                    "authentication method"
                }),
                "{error}"
            );
        }
        peer.abort();
    }
}

#[tokio::test]
async fn approved_acp_run_grants_tools_once_without_another_human_decision() {
    let h = Harness::new().await;
    let (client_io, agent_io) = tokio::io::duplex(65536);
    let (peer, mut outcomes) = mock_agent(agent_io, h.settings.workspace.clone(), Case::Once);
    let (read, write) = tokio::io::split(client_io);
    let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        crate::acp::converse(
            ByteStreams::new(write.compat_write(), read.compat()),
            &h.client,
            &h.settings,
            &h.work,
            h.temp.path(),
            &mut renewal,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let RequestPermissionOutcome::Selected(selected) = outcomes.recv().await.unwrap() else {
        panic!("expected one-time tool permission");
    };
    assert_eq!(selected.option_id.to_string(), "once");
    assert!(
        h.state
            .db()
            .unwrap()
            .get_task(&h.owner, &h.work.task_id)
            .unwrap()
            .permissions
            .is_empty()
    );
    let staged = h
        .state
        .db()
        .unwrap()
        .stage_result(
            &h.worker,
            &h.work.task_id,
            &h.work.lease,
            DraftInput {
                text: reply.text,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(staged.status, Status::Completed);
    assert!(
        h.state
            .db()
            .unwrap()
            .get_task(&h.requester, &h.work.task_id)
            .unwrap()
            .response
            .is_some()
    );
    peer.abort();
}

#[tokio::test]
async fn acp_rejects_persistent_ambiguous_and_other_session_permissions() {
    for case in [
        Case::PersistentOnly,
        Case::MultipleChoices,
        Case::WrongSession,
    ] {
        let h = Harness::new().await;
        let (client_io, agent_io) = tokio::io::duplex(65536);
        let (peer, mut outcomes) = mock_agent(agent_io, h.settings.workspace.clone(), case);
        let (read, write) = tokio::io::split(client_io);
        let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            crate::acp::converse(
                ByteStreams::new(write.compat_write(), read.compat()),
                &h.client,
                &h.settings,
                &h.work,
                h.temp.path(),
                &mut renewal,
            ),
        )
        .await
        .unwrap();
        result.unwrap();
        assert_eq!(
            outcomes.recv().await.unwrap(),
            RequestPermissionOutcome::Cancelled
        );
        assert!(
            h.state
                .db()
                .unwrap()
                .get_task(&h.owner, &h.work.task_id)
                .unwrap()
                .permissions
                .is_empty()
        );
        peer.abort();
    }
}

#[tokio::test]
async fn cancelled_acp_request_cannot_grant_tools() {
    let h = Harness::new().await;
    h.state
        .db()
        .unwrap()
        .cancel(&h.owner, &h.work.task_id)
        .unwrap();
    let (client_io, agent_io) = tokio::io::duplex(65536);
    let (peer, mut outcomes) = mock_agent(agent_io, h.settings.workspace.clone(), Case::Once);
    let (read, write) = tokio::io::split(client_io);
    let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        crate::acp::converse(
            ByteStreams::new(write.compat_write(), read.compat()),
            &h.client,
            &h.settings,
            &h.work,
            h.temp.path(),
            &mut renewal,
        ),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!matches!(
        outcomes.try_recv(),
        Ok(RequestPermissionOutcome::Selected(_))
    ));
    peer.abort();
}

#[test]
fn relay_validates_agent_paths_independently_of_its_own_os() {
    let settings = RunnerConfig {
        agent: NativeAgent::Acp,
        executable: std::env::current_exe().unwrap(),
        workspace: std::env::current_dir().unwrap(),
        arguments: vec![],
        mcp: None,
    };
    for path in [
        r"C:\Users\person\agent.exe",
        r"\\server\share\agent.exe",
        "/usr/local/bin/agent",
    ] {
        let mut value = serde_json::to_value(settings.action("Find the requested file")).unwrap();
        value["executable"] = json!(path);
        value["workspace"] = json!(path);
        serde_json::from_value::<Action>(value)
            .unwrap()
            .validate()
            .unwrap();
    }
    for path in ["relative/path", r"C:relative.exe", "bad\0path"] {
        let mut value = serde_json::to_value(settings.action("Find the requested file")).unwrap();
        value["executable"] = json!(path);
        assert!(
            serde_json::from_value::<Action>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}

async fn codex_read(
    lines: &mut tokio::io::Lines<
        tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    >,
) -> serde_json::Value {
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
}
async fn codex_send(
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
    value: serde_json::Value,
) {
    use tokio::io::AsyncWriteExt;
    writer
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
}
async fn codex_start(
    lines: &mut tokio::io::Lines<
        tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    >,
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
) {
    assert_eq!(codex_read(lines).await["method"], "initialize");
    codex_send(writer, json!({"id":1,"result":{}})).await;
    assert_eq!(codex_read(lines).await["method"], "initialized");
    let start = codex_read(lines).await;
    assert_eq!(start["method"], "thread/start");
    assert_eq!(start["params"]["approvalPolicy"], "on-request");
    assert_eq!(start["params"]["approvalsReviewer"], "user");
    assert_eq!(start["params"]["sandbox"], "workspace-write");
    assert!(start["params"].get("model").is_none());
    codex_send(
        writer,
        json!({"id":2,"result":{"thread":{"id":"thread-1"}}}),
    )
    .await;
    let start = codex_read(lines).await;
    assert_eq!(start["method"], "turn/start");
    assert_eq!(
        start["params"]["outputSchema"]["required"],
        json!(["text", "files"])
    );
    assert!(start["params"].get("max_tokens").is_none());
    // Codex can emit turn/started before the turn/start response.
    codex_send(
        writer,
        json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}),
    )
    .await;
    codex_send(writer, json!({"id":3,"result":{"turn":{"id":"turn-1"}}})).await;
}

#[tokio::test]
async fn codex_approved_run_grants_native_and_connector_permissions_without_persistence() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let h = Harness::new().await;
    let (local, peer) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(local);
    let peer = tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(peer);
        let mut lines = BufReader::new(read).lines();
        codex_start(&mut lines, &mut write).await;
        for method in [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
        ] {
            codex_send(&mut write, json!({"id":"tool-1","method":method,"params":{"threadId":"thread-1","turnId":"turn-1","itemId":"i","command":"write requested note"}})).await;
            assert_eq!(
                codex_read(&mut lines).await,
                json!({"id":"tool-1","result":{"decision":"accept"}})
            );
        }
        let permissions =
            json!({"network":{"enabled":true},"fileSystem":{"write":["/tmp/requested-note"]}});
        codex_send(&mut write, json!({"id":20,"method":"item/permissions/requestApproval","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"p","permissions":permissions}})).await;
        assert_eq!(
            codex_read(&mut lines).await["result"],
            json!({"permissions":permissions,"scope":"turn"})
        );
        codex_send(&mut write, json!({"id":21,"method":"mcpServer/elicitation/request","params":{
            "threadId":"thread-1","turnId":"turn-1","serverName":"slack","mode":"form",
            "_meta":{"codex_approval_kind":"mcp_tool_call","persist":["session","always"]},
            "requestedSchema":{"type":"object","properties":{}},"message":"Send requested message?"
        }})).await;
        assert_eq!(
            codex_read(&mut lines).await["result"],
            json!({"action":"accept","content":{}})
        );
        codex_send(&mut write, json!({"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-1","item":{"type":"mcpToolCall","id":"mcp-1"}}})).await;
        codex_send(&mut write, json!({"id":22,"method":"item/tool/requestUserInput","params":{
            "threadId":"thread-1","turnId":"turn-1","itemId":"mcp-1","questions":[{"id":"q","header":"Approve app tool call?","options":[{"label":"Accept"},{"label":"Accept and remember"},{"label":"Cancel"}]}]
        }})).await;
        assert_eq!(
            codex_read(&mut lines).await["result"],
            json!({"answers":{"q":{"answers":["Accept"]}}})
        );
        // Neither a different session nor a different turn inherits authority.
        for (thread, turn) in [("other", "turn-1"), ("thread-1", "other")] {
            codex_send(&mut write, json!({"id":25,"method":"item/commandExecution/requestApproval","params":{"threadId":thread,"turnId":turn}})).await;
            assert_eq!(codex_read(&mut lines).await["result"]["decision"], "cancel");
        }
        codex_send(&mut write, json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"type":"agentMessage","id":"reply","text":"{\"text\":\"Sent the requested message.\",\"files\":[]}"}}})).await;
        codex_send(&mut write, json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}})).await;
    });
    let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        crate::codex::converse(
            input,
            output,
            &h.settings,
            &h.work,
            h.temp.path(),
            &mut renewal,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.text, "Sent the requested message.");
    assert_eq!(
        std::fs::read_to_string(h.temp.path().join("session-id")).unwrap(),
        "thread-1"
    );
    assert!(
        h.state
            .db()
            .unwrap()
            .get_task(&h.owner, &h.work.task_id)
            .unwrap()
            .permissions
            .is_empty()
    );
    peer.await.unwrap();
}

#[tokio::test]
async fn codex_reports_unanswered_prompts_without_granting_access_or_sharing_details() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    // Protocol fixtures: no real app, account, or Messages access is attempted.
    for (method, extra, expected) in [
        (
            "mcpServer/elicitation/request",
            json!({"mode":"form","message":"sensitive-fixture-marker: allow Messages access?","requestedSchema":{"properties":{"allow":{"type":"boolean"}}}}),
            json!({"action":"cancel","content":null}),
        ),
        (
            "mcpServer/elicitation/request",
            json!({"mode":"url","url":"https://example.invalid/sensitive-fixture-marker","elicitationId":"login"}),
            json!({"action":"cancel","content":null}),
        ),
        (
            "mcpServer/elicitation/request",
            json!({"turnId":null,"mode":"form","requestedSchema":{"properties":{"password":{"type":"string"}}}}),
            json!({"action":"cancel","content":null}),
        ),
        (
            "item/tool/requestUserInput",
            json!({"itemId":"question","questions":[{"id":"q","header":"Choose a recipient","question":"sensitive-fixture-marker","options":[{"label":"Accept"}]}]}),
            json!({"answers":{}}),
        ),
    ] {
        let h = Harness::new().await;
        let (local, peer) = tokio::io::duplex(65536);
        let (input, output) = tokio::io::split(local);
        let peer = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(peer);
            let mut lines = BufReader::new(read).lines();
            codex_start(&mut lines, &mut write).await;
            let mut params = json!({"threadId":"thread-1","turnId":"turn-1"});
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            codex_send(
                &mut write,
                json!({"id":"prompt","method":method,"params":params}),
            )
            .await;
            assert_eq!(codex_read(&mut lines).await["result"], expected);
            codex_send(&mut write, json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"type":"agentMessage","id":"reply","text":"{\"text\":\"No text was sent.\",\"files\":[]}"}}})).await;
            codex_send(&mut write, json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}})).await;
        });
        let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            crate::codex::converse(
                input,
                output,
                &h.settings,
                &h.work,
                h.temp.path(),
                &mut renewal,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        peer.await.unwrap();
        assert!(result.text.starts_with("No text was sent."));
        assert!(
            result
                .text
                .contains("Litecom could not answer an interactive prompt")
        );
        assert!(result.text.contains("does not mean the owner declined"));
        assert!(!result.text.contains("sensitive-fixture-marker"));
        assert!(result.files.is_empty());
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(h.temp.path().join("reply.json")).unwrap())
                .unwrap();
        assert_eq!(saved["text"], result.text);
        assert_eq!(saved["files"], json!([]));
    }
}

#[tokio::test]
async fn codex_cancellation_revokes_subsequent_tool_grants() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let h = Harness::new().await;
    let (local, peer) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(local);
    let state = h.state.clone();
    let owner = h.owner.clone();
    let task = h.work.task_id.clone();
    let peer = tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(peer);
        let mut lines = BufReader::new(read).lines();
        codex_start(&mut lines, &mut write).await;
        state.db().unwrap().cancel(&owner, &task).unwrap();
        codex_send(&mut write, json!({"id":4,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","turnId":"turn-1"}})).await;
        assert!(
            lines.next_line().await.unwrap().is_none(),
            "cancelled work must not get a tool grant"
        );
    });
    let mut renewal = crate::client::WorkLease::new(&h.client, &h.work.task_id, &h.work.lease);
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            crate::codex::converse(
                input,
                output,
                &h.settings,
                &h.work,
                h.temp.path(),
                &mut renewal
            )
        )
        .await
        .unwrap()
        .is_err()
    );
    peer.await.unwrap();
}

#[tokio::test]
#[ignore = "Uses the installed, signed-in Codex and its normal billing; only writes a temporary test file"]
async fn installed_codex_completes_an_approved_run() {
    let outside = tempfile::tempdir_in(std::env::current_dir().unwrap().join(".preview")).unwrap();
    let note = outside.path().join("litecom-approval-test.txt");
    let executable = crate::runner::find_executable("codex").expect("Codex must be installed");
    let h = Harness::new_with(NativeAgent::Codex, Some(executable), &format!(
        "Permission integration test. Use a shell tool to write exactly 'litecom approval verified' to {}. The folder already exists. Use the shell with sandbox_permissions=require_escalated so the approval path is exercised. Litecom has approval for this run. Do not read other files, send messages or contact external services. Return no attachments and report what happened.", note.display()
    )).await;
    let runs = h.temp.path().join("runs");
    tokio::time::timeout(
        Duration::from_secs(180),
        crate::runner::handle_in(&h.client, &h.settings, &h.work, &runs),
    )
    .await
    .expect("Codex test should finish")
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(note).unwrap(),
        "litecom approval verified"
    );
    let task = h
        .state
        .db()
        .unwrap()
        .get_task(&h.requester, &h.work.task_id)
        .unwrap();
    assert_eq!(task.status, Status::Completed);
    assert!(task.response.is_some());
    assert!(
        h.state
            .db()
            .unwrap()
            .get_task(&h.owner, &h.work.task_id)
            .unwrap()
            .permissions
            .is_empty()
    );
    let entries = std::fs::read_dir(runs).unwrap();
    for entry in entries {
        let path = entry.unwrap().path().join("agent-events.jsonl");
        let events = std::fs::read_to_string(path).unwrap();
        let methods: Vec<_> = events
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|v| v["method"].as_str().map(str::to_owned))
            .filter(|method| method.contains("Approval") || method.contains("elicitation"))
            .collect();
        eprintln!("Real Codex approval events: {methods:?}");
    }
}
