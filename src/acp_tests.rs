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
        let f = Fixture::new();
        let temp = tempfile::tempdir().unwrap();
        let settings = RunnerConfig {
            agent: NativeAgent::Acp,
            executable: std::env::current_exe().unwrap(),
            workspace: temp.path().into(),
            arguments: vec![],
            mcp: Some(McpConnection {
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
        let task = f.request("acp");
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
    async fn permission(&self) -> ToolPermission {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let task = self
                    .state
                    .db()
                    .unwrap()
                    .get_task(&self.owner, &self.work.task_id)
                    .unwrap();
                if let Some(permission) = task.permissions.first() {
                    break permission.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("ACP permission reached owner")
    }
}

#[derive(Clone, Copy)]
enum Case {
    Once,
    PersistentOnly,
    MultipleChoices,
    WrongSession,
    Changed,
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
                        if matches!(case, Case::Changed) {
                            let updates = cx.clone();
                            cx.spawn(async move {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                updates.send_notification(SessionNotification::new(
                                    "fixture-session",
                                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                                        "write-1",
                                        ToolCallUpdateFields::new().raw_input(
                                            json!({"path":"other.txt","content":"changed"}),
                                        ),
                                    )),
                                ))
                            })?;
                        }
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
async fn acp_exact_permission_uses_once_and_delivers_the_approved_result() {
    for decision in [Decision::Approve, Decision::Decline] {
        let h = Harness::new().await;
        let (client_io, agent_io) = tokio::io::duplex(65536);
        let (peer, mut outcomes) = mock_agent(agent_io, h.settings.workspace.clone(), Case::Once);
        let (read, write) = tokio::io::split(client_io);
        let run = crate::acp::converse(
            ByteStreams::new(write.compat_write(), read.compat()),
            &h.client,
            &h.settings,
            &h.work,
            h.temp.path(),
        );
        tokio::pin!(run);
        let review = async {
            let p = h.permission().await;
            assert_eq!(p.details["operation"]["rawInput"]["path"], "note.txt");
            let private = h
                .state
                .db()
                .unwrap()
                .get_task(&h.requester, &h.work.task_id)
                .unwrap();
            assert!(private.permissions.is_empty() && private.response.is_none());
            assert!(outcomes.try_recv().is_err());
            h.state
                .db()
                .unwrap()
                .decide_permission(
                    &h.owner,
                    &h.work.task_id,
                    &p.id,
                    &p.digest,
                    decision.clone(),
                )
                .unwrap();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(8), async {
            tokio::join!(&mut run, review)
        })
        .await
        .unwrap();
        let reply = result.unwrap();
        assert_eq!(reply.text, "Here is the completed reply.");
        let outcome = outcomes.recv().await.unwrap();
        let RequestPermissionOutcome::Selected(selected) = outcome else {
            panic!("expected selected option")
        };
        assert_eq!(
            selected.option_id.to_string(),
            if decision == Decision::Approve {
                "once"
            } else {
                "no"
            }
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
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            crate::acp::converse(
                ByteStreams::new(write.compat_write(), read.compat()),
                &h.client,
                &h.settings,
                &h.work,
                h.temp.path(),
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
async fn acp_stops_on_scope_change_or_cancelled_request() {
    for cancelled in [false, true] {
        let h = Harness::new().await;
        let (client_io, agent_io) = tokio::io::duplex(65536);
        let (peer, mut outcomes) = mock_agent(
            agent_io,
            h.settings.workspace.clone(),
            if cancelled { Case::Once } else { Case::Changed },
        );
        let (read, write) = tokio::io::split(client_io);
        let run = crate::acp::converse(
            ByteStreams::new(write.compat_write(), read.compat()),
            &h.client,
            &h.settings,
            &h.work,
            h.temp.path(),
        );
        let review = async {
            let p = h.permission().await;
            if cancelled {
                h.state
                    .db()
                    .unwrap()
                    .cancel(&h.owner, &h.work.task_id)
                    .unwrap();
            } else {
                h.state
                    .db()
                    .unwrap()
                    .decide_permission(
                        &h.owner,
                        &h.work.task_id,
                        &p.id,
                        &p.digest,
                        Decision::Approve,
                    )
                    .unwrap();
            }
        };
        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(8), async { tokio::join!(run, review) })
                .await
                .unwrap();
        assert!(result.is_err());
        assert!(!matches!(
            outcomes.try_recv(),
            Ok(RequestPermissionOutcome::Selected(_))
        ));
        peer.abort();
    }
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
