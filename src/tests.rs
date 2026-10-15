use crate::{auth::Session, crypto::*, model::*, store::*};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

#[path = "cloud_tests.rs"]
mod cloud_tests;

#[path = "acp_tests.rs"]
mod acp_tests;

#[path = "backup_tests.rs"]
mod backup_tests;

#[path = "reliability_tests.rs"]
mod reliability_tests;

fn user(store: &Store, name: &str, phone: &str) -> (Principal, Session) {
    let (challenge, code) = store.begin_login(name, phone, "browser").unwrap();
    let session = store.verify_login(&challenge.id, &code, "browser").unwrap();
    (
        store.authenticate(&session.token, Role::Human).unwrap(),
        session,
    )
}
fn worker(store: &Store, human: &Principal, agent: &str) -> Principal {
    let device = store.begin_device("Test worker").unwrap();
    store
        .authorize_device(human, &device.user_code, agent)
        .unwrap();
    let token = store.poll_device(&device.device_code).unwrap().unwrap();
    store.authenticate(&token.token, Role::Agent).unwrap()
}
struct Fixture {
    store: Store,
    a: Principal,
    b: Principal,
    bw: Principal,
    agent: String,
    clock: Arc<AtomicI64>,
}
impl Fixture {
    fn new() -> Self {
        let clock = Arc::new(AtomicI64::new(1000));
        let clone = clock.clone();
        let store = Store::with_clock(
            ":memory:",
            [7; 32],
            Arc::new(move || clone.load(Ordering::SeqCst)),
        )
        .unwrap();
        let (a, _) = user(&store, "Theo", "+14155550101");
        let (b, profile) = user(&store, "Moe", "+14155550102");
        store.request_contact(&a, &profile.profile.invite).unwrap();
        store.accept_contact(&b, &a.user_id).unwrap();
        let agent = store.agents(&b, &b.user_id).unwrap()[0].id.clone();
        let bw = worker(&store, &b, &agent);
        Self {
            store,
            a,
            b,
            bw,
            agent,
            clock,
        }
    }
    fn request(&self, key: &str) -> TaskView {
        self.store
            .create_task(
                &self.a,
                NewTask {
                    owner_id: self.b.user_id.clone(),
                    agent_id: None,
                    request: "Please send the document".into(),
                    idempotency_key: key.into(),
                },
            )
            .unwrap()
    }
    fn draft(&self, key: &str, action: Option<Action>) -> TaskView {
        let task = self.request(key);
        let work = self.store.claim(&self.bw, &self.agent).unwrap().unwrap();
        self.store
            .prepare(
                &self.bw,
                &task.id,
                Some(&work.lease),
                DraftInput {
                    text: "A private draft".into(),
                    attachment_ids: vec![],
                    action,
                },
            )
            .unwrap()
    }
    fn approve(&self, task: &TaskView) -> TaskView {
        let draft = task.draft.as_ref().unwrap();
        self.store
            .decide(
                &self.b,
                &task.id,
                draft.version,
                &draft.digest,
                Decision::Approve,
            )
            .unwrap()
    }
    fn advance(&self, seconds: i64) {
        self.clock.fetch_add(seconds, Ordering::SeqCst);
    }
}
fn action() -> Action {
    Action::Command {
        title: "Create branch".into(),
        description: "Create a branch in the selected repository.".into(),
        argv: vec!["git".into(), "branch".into(), "new".into()],
        cwd: "/tmp".into(),
        acting_account: "Moe's local account".into(),
    }
}

#[test]
fn encryption_authenticates_context_and_hides_phone() {
    let vault = Vault::new([5; 32]);
    let secret = b"private body";
    let one = vault.encrypt(secret, "one").unwrap();
    let two = vault.encrypt(secret, "one").unwrap();
    assert_ne!(one, two);
    assert_eq!(vault.decrypt(&one, "one").unwrap(), secret);
    assert!(vault.decrypt(&one, "two").is_err());
    let mut corrupt = one;
    corrupt[14] ^= 1;
    assert!(vault.decrypt(&corrupt, "one").is_err());
}
#[test]
fn identity_survives_login_and_each_agent_credential_is_revocable() {
    let f = Fixture::new();
    f.advance(61);
    let (again, _) = user(&f.store, "A changed label", "+14155550102");
    assert_eq!(again.user_id, f.b.user_id);
    let other = worker(&f.store, &f.b, &f.agent);
    assert_ne!(f.bw.credential_id, other.credential_id);
    f.store
        .revoke_connection(&f.b, &f.bw.credential_id)
        .unwrap();
    let active: i64 = f
        .store
        .db
        .query_row(
            "SELECT COUNT(*) FROM credentials WHERE role='agent' AND revoked=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 1);
    let phone: String = f.store.phone(&f.b.user_id).unwrap();
    assert_eq!(phone, "+14155550102");
    let cipher: Vec<u8> = f
        .store
        .db
        .query_row(
            "SELECT phone FROM profiles WHERE id=?",
            [&f.b.user_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!cipher.windows(phone.len()).any(|v| v == phone.as_bytes()));
}
#[test]
fn phone_codes_are_browser_bound_single_use_and_attempt_limited() {
    let store = Store::open(":memory:", [4; 32]).unwrap();
    let (c, code) = store
        .begin_login("Theo", "+14155550101", "browser-one")
        .unwrap();
    assert!(store.verify_login(&c.id, &code, "browser-two").is_err());
    for _ in 0..4 {
        assert!(store.verify_login(&c.id, "wrong", "browser-one").is_err());
    }
    assert!(store.verify_login(&c.id, &code, "browser-one").is_err());
    let (c, code) = store
        .begin_login("Moe", "+14155550102", "browser-one")
        .unwrap();
    let s = store.verify_login(&c.id, &code, "browser-one").unwrap();
    assert!(store.verify_login(&c.id, &code, "browser-one").is_err());
    assert!(store.authenticate(&s.token, Role::Agent).is_err());
}
#[test]
fn pairing_cannot_elevate_or_change_owner() {
    let f = Fixture::new();
    let device = f.store.begin_device("Claude").unwrap();
    assert!(f.store.poll_device(&device.device_code).unwrap().is_none());
    assert!(
        f.store
            .authorize_device(&f.bw, &device.user_code, &f.agent)
            .is_err()
    );
    assert!(
        f.store
            .authorize_device(&f.a, &device.user_code, &f.agent)
            .is_err()
    );
    f.store
        .authorize_device(&f.b, &device.user_code, &f.agent)
        .unwrap();
    let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
    assert!(f.store.poll_device(&device.device_code).is_err());
    assert!(f.store.authenticate(&token.token, Role::Human).is_err());
    assert!(f.store.check_csrf(&f.b, "wrong").is_err());
}
#[test]
fn mutual_contacts_required_and_names_cannot_override_identity() {
    let f = Fixture::new();
    let (c, s) = user(&f.store, "Moe", "+14155550103");
    assert!(
        f.store
            .create_task(
                &f.a,
                NewTask {
                    owner_id: c.user_id.clone(),
                    agent_id: None,
                    request: "hi".into(),
                    idempotency_key: "1".into()
                }
            )
            .is_err()
    );
    f.store.request_contact(&f.a, &s.profile.invite).unwrap();
    assert!(f.store.accept_contact(&f.a, &c.user_id).is_err());
    f.store.accept_contact(&c, &f.a.user_id).unwrap();
    assert!(f.store.resolve(&f.a, "Moe", None).is_err());
    assert_eq!(
        f.store.resolve(&f.a, &f.b.user_id, None).unwrap().0,
        f.b.user_id
    );
}
#[test]
fn self_requests_and_named_cloud_routing_work_without_contacts() {
    let f = Fixture::new();
    let devin = f
        .store
        .register_agent(&f.b, "Devin", AgentKind::Devin, false)
        .unwrap();
    assert_eq!(
        f.store.resolve(&f.bw, "me", Some("devin")).unwrap(),
        (f.b.user_id.clone(), devin.id.clone())
    );
    let own = f
        .store
        .create_task(
            &f.bw,
            NewTask {
                owner_id: f.b.user_id.clone(),
                agent_id: Some(devin.id.clone()),
                request: "Run my tests".into(),
                idempotency_key: id(),
            },
        )
        .unwrap();
    assert_eq!(own.agent_id, devin.id);
    let first = f.request("stable");
    f.store.set_default(&f.b, &devin.id).unwrap();
    assert_eq!(f.request("stable").agent_id, first.agent_id);
    assert_eq!(f.request("new").agent_id, devin.id);
}
#[test]
fn drafts_and_snapshot_bytes_remain_private_until_exact_approval() {
    let f = Fixture::new();
    let task = f.request("one");
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let file = f
        .store
        .upload(
            &f.bw,
            &task.id,
            Some(&work.lease),
            "report.pdf",
            "application/pdf",
            b"original bytes",
        )
        .unwrap();
    let draft = f
        .store
        .prepare(
            &f.bw,
            &task.id,
            Some(&work.lease),
            DraftInput {
                text: "Here you go".into(),
                attachment_ids: vec![file.id.clone()],
                action: None,
            },
        )
        .unwrap();
    let view = f.store.get_task(&f.a, &task.id).unwrap();
    assert!(view.draft.is_none() && view.response.is_none());
    assert!(f.store.download(&f.a, &file.id).is_err());
    assert!(
        f.store
            .decide(
                &f.bw,
                &task.id,
                1,
                &draft.draft.as_ref().unwrap().digest,
                Decision::Approve
            )
            .is_err()
    );
    f.approve(&draft);
    let response = f.store.get_task(&f.a, &task.id).unwrap();
    assert_eq!(response.response.unwrap().text, "Here you go");
    let (meta, bytes) = f.store.download(&f.a, &file.id).unwrap();
    assert_eq!(bytes, b"original bytes");
    assert_eq!(meta.sha256, hash(&bytes));
    assert!(
        f.store
            .prepare(
                &f.b,
                &task.id,
                None,
                DraftInput {
                    text: "changed".into(),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert!(f.store.acknowledge(&f.b, &task.id).is_err());
    assert!(
        f.store
            .acknowledge(&f.a, &task.id)
            .unwrap()
            .received_at
            .is_some()
    );
}
#[test]
fn stale_approval_fails_and_duplicate_decisions_are_safe() {
    let f = Fixture::new();
    let first = f.draft("one", None);
    let d = first.draft.unwrap();
    let changed = f
        .store
        .prepare(
            &f.b,
            &first.id,
            None,
            DraftInput {
                text: "Edited".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        f.store
            .decide(&f.b, &first.id, d.version, &d.digest, Decision::Approve)
            .is_err()
    );
    f.approve(&changed);
    f.approve(&changed);
    let d = changed.draft.unwrap();
    assert!(
        f.store
            .decide(&f.b, &first.id, d.version, &d.digest, Decision::Decline)
            .is_err()
    );
    let approvals: i64 = f
        .store
        .db
        .query_row("SELECT COUNT(*) FROM approvals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(approvals, 1);
}
#[test]
fn attachments_cannot_cross_requests_or_appear_after_approval() {
    let f = Fixture::new();
    let task = f.request("one");
    let extra = f
        .store
        .upload(
            &f.b,
            &task.id,
            None,
            "secret",
            "text/plain",
            b"not approved",
        )
        .unwrap();
    let two = f.request("two");
    assert!(
        f.store
            .prepare(
                &f.b,
                &two.id,
                None,
                DraftInput {
                    text: "hi".into(),
                    attachment_ids: vec![extra.id.clone()],
                    action: None
                }
            )
            .is_err()
    );
    let draft = f
        .store
        .prepare(
            &f.b,
            &task.id,
            None,
            DraftInput {
                text: "No file".into(),
                ..Default::default()
            },
        )
        .unwrap();
    f.approve(&draft);
    assert!(f.store.download(&f.a, &extra.id).is_err());
    assert!(
        f.store
            .upload(&f.b, &task.id, None, "late", "text/plain", b"late")
            .is_err()
    );
}
#[test]
fn lease_recovery_retries_preparation_but_never_execution() {
    let f = Fixture::new();
    let task = f.request("one");
    let old = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    f.advance(LEASE_TTL + 1);
    let next = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    assert_eq!(old.task_id, next.task_id);
    assert!(
        f.store
            .prepare(
                &f.bw,
                &task.id,
                Some(&old.lease),
                DraftInput {
                    text: "old".into(),
                    ..Default::default()
                }
            )
            .is_err()
    );
    let draft = f
        .store
        .prepare(
            &f.bw,
            &task.id,
            Some(&next.lease),
            DraftInput {
                text: "Approve this operation".into(),
                attachment_ids: vec![],
                action: Some(action()),
            },
        )
        .unwrap();
    f.approve(&draft);
    let exec = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    assert_eq!(exec.phase, "execute");
    f.advance(LEASE_TTL + 1);
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    let view = f.store.get_task(&f.a, &task.id).unwrap();
    assert_eq!(view.status, Status::Failed);
    assert_eq!(view.receipt.unwrap().outcome, Outcome::Uncertain);
}
#[test]
fn failures_visible_but_private_tool_output_is_not_auto_shared() {
    let f = Fixture::new();
    let draft = f.draft("one", Some(action()));
    f.approve(&draft);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    f.store
        .finish(
            &f.bw,
            &draft.id,
            &work.lease,
            Receipt {
                outcome: Outcome::Failed,
                summary: "secret token from stdout".into(),
                urls: vec!["https://private.example.com/secret".into()],
            },
        )
        .unwrap();
    let view = f.store.get_task(&f.a, &draft.id).unwrap();
    let receipt = view.receipt.unwrap();
    assert_eq!(receipt.outcome, Outcome::Failed);
    assert!(!receipt.summary.contains("secret"));
    assert!(receipt.urls.is_empty());
}
#[test]
fn cancellation_and_blocking_revoke_future_access() {
    let f = Fixture::new();
    let task = f.draft("one", None);
    f.approve(&task);
    f.store.block_contact(&f.b, &f.a.user_id).unwrap();
    let view = f.store.get_task(&f.a, &task.id).unwrap();
    assert!(view.revoked_at.is_some() && view.response.is_none());
    assert!(f.store.resolve(&f.a, "Moe", None).is_err());
    let f = Fixture::new();
    let task = f.draft("two", Some(action()));
    f.approve(&task);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    assert_eq!(
        f.store.cancel(&f.a, &task.id).unwrap().status,
        Status::CancelRequested
    );
    assert_eq!(
        f.store.heartbeat(&f.bw, &task.id, &work.lease).unwrap(),
        Status::CancelRequested
    );
}
#[test]
fn persisted_state_survives_restart_and_wrong_key_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path, [3; 32]).unwrap();
    let (a, s) = user(&store, "Theo", "+14155550101");
    let task = store
        .create_task(
            &a,
            NewTask {
                owner_id: a.user_id.clone(),
                agent_id: None,
                request: "my task".into(),
                idempotency_key: "durable".into(),
            },
        )
        .unwrap();
    drop(store);
    assert!(Store::open(&path, [4; 32]).is_err());
    let store = Store::open(&path, [3; 32]).unwrap();
    let a = store.authenticate(&s.token, Role::Human).unwrap();
    assert_eq!(store.get_task(&a, &task.id).unwrap().request, "my task");
}
#[test]
fn pagination_does_not_skip_equal_timestamps_and_idempotency_rejects_change() {
    let f = Fixture::new();
    for i in 0..105 {
        f.request(&i.to_string());
    }
    let first = f.store.tasks(&f.a, None).unwrap();
    assert_eq!(first.len(), 100);
    let last = first.last().unwrap();
    let second = f
        .store
        .tasks(&f.a, Some((last.created_at, last.id.clone())))
        .unwrap();
    assert_eq!(second.len(), 5);
    assert!(first.iter().all(|t| !second.iter().any(|s| s.id == t.id)));
    assert!(
        f.store
            .create_task(
                &f.a,
                NewTask {
                    owner_id: f.b.user_id.clone(),
                    agent_id: None,
                    request: "changed".into(),
                    idempotency_key: "0".into()
                }
            )
            .is_err()
    );
}

#[tokio::test]
async fn http_auth_requires_human_cookie_origin_and_csrf_for_approval() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = Fixture::new();
    let draft = f.draft("http", None);
    let d = draft.draft.unwrap();
    f.advance(61);
    let (_, session) = user(&f.store, "Moe", "+14155550102");
    let state = crate::api::AppState::new(
        f.store,
        "http://127.0.0.1:4141".into(),
        crate::providers::Providers::from_env(true).unwrap(),
    );
    let app = crate::api::router(state);
    let payload =
        serde_json::json!({"version":d.version,"digest":d.digest,"decision":"approve"}).to_string();
    let path = format!("/api/tasks/{}/decision", draft.id);
    for (origin, csrf, expected) in [
        (None, Some(session.csrf.as_str()), StatusCode::FORBIDDEN),
        (
            Some("https://evil.example"),
            Some(session.csrf.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (Some("http://127.0.0.1:4141"), None, StatusCode::FORBIDDEN),
        (
            Some("http://127.0.0.1:4141"),
            Some(session.csrf.as_str()),
            StatusCode::OK,
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .header("cookie", format!("litecom_session={}", session.token));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if let Some(csrf) = csrf {
            request = request.header("x-csrf-token", csrf);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(payload.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().contains_key("content-security-policy"));
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_sdk_negotiates_and_exposes_no_approval_tool() {
    use rmcp::{ServiceExt, model::ClientConfig};
    let config = crate::config::ClientConfig {
        relay: "http://127.0.0.1:4141".into(),
        token: "fixture".into(),
        user_id: id(),
        agent_id: id(),
        workspace: None,
        runner: None,
        approval_channel: None,
    };
    let (client_side, server_side) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        crate::mcp::LitecomMcp::new(crate::client::Client::new(config).unwrap())
            .serve(server_side)
            .await
            .unwrap()
    });
    let client = ClientConfig::default().serve(client_side).await.unwrap();
    let server = server.await.unwrap();
    let tools = client.list_tools(Default::default()).await.unwrap().tools;
    assert_eq!(tools.len(), 8);
    assert!(tools.iter().any(|t| t.name == "litecom_ask"));
    assert!(
        tools
            .iter()
            .all(|t| !t.name.contains("approve") && !t.name.contains("decision"))
    );
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}

#[tokio::test]
async fn real_client_uses_a2a_and_preserves_private_content_and_routing() {
    use a2a_protocol_types::{
        agent_card::AgentCard,
        task::{Task, TaskState},
    };
    use serde_json::json;
    let f = Fixture::new();
    let agent = f.store.agents(&f.a, &f.a.user_id).unwrap()[0].id.clone();
    let device = f.store.begin_device("A2A client").unwrap();
    f.store
        .authorize_device(&f.a, &device.user_code, &agent)
        .unwrap();
    let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let config = crate::config::ClientConfig {
        relay: origin.clone(),
        token: token.token.clone(),
        user_id: f.a.user_id.clone(),
        agent_id: agent,
        workspace: None,
        runner: None,
        approval_channel: None,
    };
    let state = crate::api::AppState::new(
        f.store,
        origin.clone(),
        crate::providers::Providers::from_env(true).unwrap(),
    );
    let router = crate::api::router(state.clone());
    let handle = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = crate::client::Client::new(config).unwrap();
    let request = client
        .ask("Moe", None, "Send the report", "a2a-test")
        .await
        .unwrap();
    assert_eq!(request.status, Status::Queued);
    let remote = format!("{origin}/a2a/{}/{}", f.b.user_id, f.agent);
    let http = reqwest::Client::new();
    let card: AgentCard = http
        .get(format!("{remote}/.well-known/agent-card.json"))
        .bearer_auth(&token.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card.supported_interfaces[0].protocol_version, "1.0");
    let draft = state
        .db()
        .unwrap()
        .prepare(
            &f.b,
            &request.id,
            None,
            DraftInput {
                text: "Private payload with a secret".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let get = json!({"jsonrpc":"2.0","id":2,"method":"GetTask","params":{"id":request.id}});
    let response: serde_json::Value = http
        .post(&remote)
        .bearer_auth(&token.token)
        .json(&get)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!response.to_string().contains("secret"));
    let task: Task = serde_json::from_value(response["result"].clone()).unwrap();
    assert_eq!(task.status.state, TaskState::Working);
    assert!(task.artifacts.is_none());
    let d = draft.draft.unwrap();
    state
        .db()
        .unwrap()
        .decide(&f.b, &request.id, d.version, &d.digest, Decision::Approve)
        .unwrap();
    let response: serde_json::Value = http
        .post(&remote)
        .bearer_auth(&token.token)
        .json(&get)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task: Task = serde_json::from_value(response["result"].clone()).unwrap();
    assert_eq!(task.status.state, TaskState::Completed);
    assert!(
        response
            .to_string()
            .contains("Private payload with a secret")
    );
    let cloud = state
        .db()
        .unwrap()
        .register_agent(&f.b, "Devin", AgentKind::Devin, true)
        .unwrap();
    let retry = client
        .ask("Moe", None, "Send the report", "a2a-test")
        .await
        .unwrap();
    assert_eq!(retry.id, request.id);
    assert_ne!(retry.agent_id, cloud.id);
    assert!(
        client
            .ask("Moe", None, "Different request", "a2a-test")
            .await
            .is_err()
    );
    handle.abort();
}

#[test]
fn execution_output_needs_its_own_immutable_sharing_approval() {
    let f = Fixture::new();
    let launch = f.draft("run", Some(action()));
    let first = launch.draft.clone().unwrap();
    f.approve(&launch);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let file = f
        .store
        .upload(
            &f.bw,
            &launch.id,
            Some(&work.lease),
            "result.txt",
            "text/plain",
            b"private result",
        )
        .unwrap();
    assert!(f.store.download(&f.a, &file.id).is_err());
    assert!(
        f.store
            .acknowledge_download(&f.a, &file.id, &file.sha256)
            .is_err()
    );
    let result = f
        .store
        .stage_result(
            &f.bw,
            &launch.id,
            &work.lease,
            DraftInput {
                text: "Private execution output".into(),
                attachment_ids: vec![file.id.clone()],
                action: None,
            },
        )
        .unwrap();
    let external = f.store.get_task(&f.a, &launch.id).unwrap();
    assert_eq!(external.status, Status::AwaitingApproval);
    assert!(external.draft.is_none() && external.response.is_none());
    assert!(
        f.store
            .decide(
                &f.b,
                &launch.id,
                first.version,
                &first.digest,
                Decision::Approve
            )
            .is_err()
    );
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    f.approve(&result);
    assert_eq!(
        f.store.download(&f.a, &file.id).unwrap().1,
        b"private result"
    );
    assert!(
        f.store
            .acknowledge_download(&f.a, &file.id, "wrong")
            .is_err()
    );
    assert!(
        f.store
            .acknowledge_download(&f.b, &file.id, &file.sha256)
            .is_err()
    );
    f.store
        .acknowledge_download(&f.a, &file.id, &file.sha256)
        .unwrap();
    let twice = f
        .store
        .acknowledge_download(&f.a, &file.id, &file.sha256)
        .unwrap();
    assert_eq!(twice.downloads.len(), 1);
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    f.store.cancel(&f.b, &launch.id).unwrap();
    assert!(
        f.store
            .acknowledge_download(&f.a, &file.id, &file.sha256)
            .is_err()
    );
}

#[test]
fn cancelled_execution_cannot_stage_output_or_chain_an_action() {
    let f = Fixture::new();
    let launch = f.draft("cancel-output", Some(action()));
    f.approve(&launch);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let another_action = DraftInput {
        text: "another operation".into(),
        action: Some(action()),
        ..Default::default()
    };
    assert!(
        f.store
            .stage_result(&f.bw, &launch.id, &work.lease, another_action)
            .is_err()
    );
    f.store.cancel(&f.a, &launch.id).unwrap();
    assert!(
        f.store
            .stage_result(
                &f.bw,
                &launch.id,
                &work.lease,
                DraftInput {
                    text: "too late".into(),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert!(
        f.store
            .upload(
                &f.bw,
                &launch.id,
                Some(&work.lease),
                "late.txt",
                "text/plain",
                b"late"
            )
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn background_runner_requires_launch_and_sharing_decisions() {
    use crate::{
        client::Client,
        config::ClientConfig,
        runner::{NativeAgent, RunnerConfig, handle_in},
    };
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    for agent_kind in [NativeAgent::Codex, NativeAgent::Claude] {
        let f = Fixture::new();
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("work space");
        std::fs::create_dir(&workspace).unwrap();
        let file = workspace.join("launch.txt");
        std::fs::write(&file, b"original file bytes").unwrap();
        let reply = json!({"text":"Here are the launch notes.","files":[file]});
        std::fs::write(
            workspace.join("reply.fixture"),
            serde_json::to_vec(&reply).unwrap(),
        )
        .unwrap();
        std::fs::write(
            workspace.join("claude.fixture"),
            serde_json::to_vec(&json!({"is_error":false,"structured_output":reply})).unwrap(),
        )
        .unwrap();
        let executable = temp.path().join("fake-agent");
        // A deterministic process fixture exercises the real subprocess and HTTP
        // boundaries. It does not contact or stand in as evidence for a live model.
        std::fs::write(
            &executable,
            r#"#!/bin/sh
cat > prompt.fixture
printf '%s\0' "$@" > arguments.fixture
printf started > invoked.fixture
if [ "$1" = exec ]; then
  while [ "$#" -gt 0 ]; do
    if [ "$1" = --output-last-message ]; then shift; cp reply.fixture "$1"; fi
    shift
  done
else
  cat claude.fixture
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let settings = RunnerConfig {
            agent: agent_kind,
            executable,
            workspace: workspace.clone(),
            arguments: vec![],
            mcp: None,
        };
        let device = f.store.begin_device("Worker fixture").unwrap();
        f.store
            .authorize_device(&f.b, &device.user_code, &f.agent)
            .unwrap();
        let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
        let principal = f.store.authenticate(&token.token, Role::Agent).unwrap();
        let request = f.request("worker");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = crate::api::AppState::new(
            f.store,
            origin.clone(),
            crate::providers::Providers::from_env(true).unwrap(),
        );
        let router = crate::api::router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = Client::new(ClientConfig {
            relay: origin,
            token: token.token,
            user_id: f.b.user_id.clone(),
            agent_id: f.agent.clone(),
            workspace: None,
            runner: None,
            approval_channel: None,
        })
        .unwrap();
        let work = state
            .db()
            .unwrap()
            .claim(&principal, &f.agent)
            .unwrap()
            .unwrap();
        handle_in(&client, &settings, &work, &temp.path().join("runs"))
            .await
            .unwrap();
        assert!(!workspace.join("invoked.fixture").exists());
        let launch = state
            .db()
            .unwrap()
            .get_task(&f.b, &request.id)
            .unwrap()
            .draft
            .unwrap();
        state
            .db()
            .unwrap()
            .decide(
                &f.b,
                &request.id,
                launch.version,
                &launch.digest,
                Decision::Approve,
            )
            .unwrap();
        let work = state
            .db()
            .unwrap()
            .claim(&principal, &f.agent)
            .unwrap()
            .unwrap();
        let mut changed = settings.clone();
        changed.workspace = temp.path().into();
        assert!(
            handle_in(&client, &changed, &work, &temp.path().join("runs"))
                .await
                .is_err()
        );
        assert!(!workspace.join("invoked.fixture").exists());
        handle_in(&client, &settings, &work, &temp.path().join("runs"))
            .await
            .unwrap();
        assert!(workspace.join("invoked.fixture").exists());
        let arguments = std::fs::read_to_string(workspace.join("arguments.fixture")).unwrap();
        let arguments: Vec<_> = arguments.split_terminator('\0').collect();
        match agent_kind {
            NativeAgent::Codex => {
                assert!(
                    arguments
                        .windows(2)
                        .any(|a| a == ["-c", "approval_policy=\"never\""])
                );
                assert!(
                    arguments
                        .windows(2)
                        .any(|a| a == ["--sandbox", "workspace-write"])
                );
                assert!(!arguments.iter().any(|a| a.starts_with("ask_for_approval=")));
            }
            NativeAgent::Claude => {
                assert!(
                    arguments
                        .windows(2)
                        .any(|a| a == ["--permission-mode", "dontAsk"])
                );
            }
            _ => unreachable!(),
        }
        assert!(!arguments.iter().any(|a| a.contains("bypass")
            || a.contains("dangerously")
            || a.contains("max-tokens")));
        let private = state.db().unwrap().get_task(&f.a, &request.id).unwrap();
        assert_eq!(private.status, Status::AwaitingApproval);
        assert!(private.response.is_none() && private.draft.is_none());
        assert!(
            state
                .db()
                .unwrap()
                .claim(&principal, &f.agent)
                .unwrap()
                .is_none()
        );
        let result = state
            .db()
            .unwrap()
            .get_task(&f.b, &request.id)
            .unwrap()
            .draft
            .unwrap();
        let file = &result.attachments[0];
        std::fs::write(workspace.join("launch.txt"), b"changed afterwards").unwrap();
        state
            .db()
            .unwrap()
            .decide(
                &f.b,
                &request.id,
                result.version,
                &result.digest,
                Decision::Approve,
            )
            .unwrap();
        assert_eq!(
            state.db().unwrap().download(&f.a, &file.id).unwrap().1,
            b"original file bytes"
        );
        if agent_kind == NativeAgent::Claude {
            // Some provider errors arrive as JSON on stdout with nonzero exit
            // status and no stderr. They must reach only the owner's receipt.
            std::fs::write(
                &settings.executable,
                b"#!/bin/sh\ncat >/dev/null\ncat claude.fixture\nexit 1\n",
            )
            .unwrap();
            std::fs::write(
                workspace.join("claude.fixture"),
                br#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: 422 Budget has been exceeded! private-account-detail"}"#,
            )
            .unwrap();
            let task = state
                .db()
                .unwrap()
                .create_task(
                    &f.a,
                    NewTask {
                        owner_id: f.b.user_id.clone(),
                        agent_id: Some(f.agent.clone()),
                        request: "Please prepare another file".into(),
                        idempotency_key: "provider-error".into(),
                    },
                )
                .unwrap();
            let prepare = state
                .db()
                .unwrap()
                .claim(&principal, &f.agent)
                .unwrap()
                .unwrap();
            handle_in(&client, &settings, &prepare, &temp.path().join("runs"))
                .await
                .unwrap();
            let draft = state
                .db()
                .unwrap()
                .get_task(&f.b, &task.id)
                .unwrap()
                .draft
                .unwrap();
            state
                .db()
                .unwrap()
                .decide(
                    &f.b,
                    &task.id,
                    draft.version,
                    &draft.digest,
                    Decision::Approve,
                )
                .unwrap();
            let execute = state
                .db()
                .unwrap()
                .claim(&principal, &f.agent)
                .unwrap()
                .unwrap();
            let error = handle_in(&client, &settings, &execute, &temp.path().join("runs"))
                .await
                .unwrap_err();
            let detail = format!("{error:#}");
            assert!(detail.contains("Budget has been exceeded"));
            state
                .db()
                .unwrap()
                .finish(
                    &principal,
                    &task.id,
                    &execute.lease,
                    Receipt {
                        outcome: Outcome::Uncertain,
                        summary: detail,
                        urls: vec![],
                    },
                )
                .unwrap();
            let owner = state.db().unwrap().get_task(&f.b, &task.id).unwrap();
            assert_eq!(owner.status, Status::Failed);
            assert!(
                owner
                    .receipt
                    .unwrap()
                    .summary
                    .contains("private-account-detail")
            );
            let requester = state.db().unwrap().get_task(&f.a, &task.id).unwrap();
            assert!(requester.response.is_none());
            assert!(
                !requester
                    .receipt
                    .unwrap()
                    .summary
                    .contains("private-account-detail")
            );
        }
        server.abort();
    }
}

#[cfg(unix)]
#[test]
fn attachments_cannot_escape_the_selected_folder_through_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let secret = temp.path().join("private.txt");
    std::fs::write(&secret, b"outside").unwrap();
    std::os::unix::fs::symlink(&secret, work.join("escape.txt")).unwrap();
    assert!(crate::runner::attachment_path(&work, &work.join("escape.txt")).is_err());
    assert!(crate::runner::attachment_path(&work, std::path::Path::new("relative.txt")).is_err());
    std::fs::write(work.join("safe.txt"), b"inside").unwrap();
    assert!(crate::runner::attachment_path(&work, &work.join("safe.txt")).is_ok());
}

#[test]
fn startup_definitions_preserve_spaces_and_do_not_embed_credentials() {
    use std::path::Path;
    let executable = Path::new("/Users/A & B/$Tools/litecom");
    let home = Path::new("/Users/A & B/config%name");
    let plist = crate::service::launch_agent(
        "work",
        executable,
        home,
        Path::new("/tmp/a.log"),
        "/usr/bin",
    )
    .unwrap();
    assert!(
        plist.contains("A &amp; B")
            && plist.contains("<string>--connection</string><string>work</string>")
    );
    let systemd = crate::service::systemd_unit("work", executable, home, "/usr/bin").unwrap();
    assert!(systemd.contains("$$Tools") && systemd.contains("config%%name"));
    assert!(!plist.contains("token") && !systemd.contains("token"));
    assert!(crate::service::systemd_unit("bad/name", executable, home, "").is_err());
}

#[test]
fn email_linking_is_human_verified_bound_to_session_and_rate_limited() {
    let f = Fixture::new();
    assert!(f.store.begin_email(&f.bw, "person@example.com").is_err());
    assert!(
        f.store
            .begin_email(&f.b, "person@example.com\r\nBcc: other@example.com")
            .is_err()
    );
    let (challenge, code) = f.store.begin_email(&f.b, "person@example.com").unwrap();
    assert!(f.store.begin_email(&f.b, "another@example.com").is_err());
    assert!(f.store.verify_email(&f.a, &challenge.id, &code).is_err());
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        assert!(f.store.verify_email(&f.b, &challenge.id, wrong).is_err());
    }
    assert!(f.store.verify_email(&f.b, &challenge.id, &code).is_err());
    assert!(f.store.channels(&f.b).unwrap().is_empty());
    f.advance(61);
    let (challenge, code) = f.store.begin_email(&f.b, "person@example.com").unwrap();
    f.store.verify_email(&f.b, &challenge.id, &code).unwrap();
    assert!(f.store.verify_email(&f.b, &challenge.id, &code).is_err());
    let channels = f.store.channels(&f.b).unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].kind, "email");
    assert!(channels[0].verified);
    // Linking a notification address never replaces the stable phone account.
    assert_eq!(f.store.profile(&f.b).unwrap().id, f.b.user_id);
}

#[test]
fn notification_outbox_survives_long_outages_and_coalesces_pending_changes() {
    let f = Fixture::new();
    f.store.enable_phone(&f.b).unwrap();
    let (challenge, code) = f.store.begin_email(&f.b, "person@example.com").unwrap();
    f.store.verify_email(&f.b, &challenge.id, &code).unwrap();
    let draft = f.draft("delivery", None);
    f.advance(3600);
    let a = f.store.next_delivery().unwrap().unwrap();
    let b = f.store.next_delivery().unwrap().unwrap();
    assert_ne!(a.channel_kind, b.channel_kind);
    assert_eq!(a.kind, "awaiting_approval");
    assert_eq!(b.kind, "awaiting_approval");
    assert_eq!(a.task_id, draft.id);
    f.store.delivery_result(&a, &Ok("sent-a".into())).unwrap();
    f.store.delivery_result(&b, &Ok("sent-b".into())).unwrap();
    assert!(f.store.next_delivery().unwrap().is_none());
    f.approve(&draft);
    let a = f.store.next_delivery().unwrap().unwrap();
    let b = f.store.next_delivery().unwrap().unwrap();
    assert_eq!(a.kind, "approved");
    assert_eq!(b.kind, "approved");
}

#[test]
fn notification_retries_follow_current_state_and_cannot_undo_disabled_channels() {
    let f = Fixture::new();
    f.store.enable_phone(&f.b).unwrap();
    let task = f.draft("notification-state", None);
    let delivery = f.store.next_delivery().unwrap().unwrap();
    assert!(
        f.store
            .delivery_title(&delivery)
            .unwrap()
            .contains("ready for your review")
    );
    f.approve(&task);
    assert!(
        f.store
            .delivery_title(&delivery)
            .unwrap()
            .contains("approved and is available")
    );
    f.store.cancel(&f.b, &task.id).unwrap();
    assert!(
        f.store
            .delivery_title(&delivery)
            .unwrap()
            .contains("revoked")
    );
    f.store.disable_channel(&f.b, &delivery.channel_id).unwrap();
    f.store
        .delivery_result(&delivery, &Ok("late-result".into()))
        .unwrap();
    let state: String = f
        .store
        .db
        .query_row(
            "SELECT state FROM deliveries WHERE event_id=? AND channel_id=?",
            rusqlite::params![delivery.event_id, delivery.channel_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "cancelled");
    f.advance(8 * 24 * 60 * 60);
    assert!(
        f.store
            .delivery_title(&delivery)
            .unwrap()
            .contains("revoked")
    );
}

#[test]
fn tool_permission_is_private_exact_and_bound_to_one_live_execution() {
    let f = Fixture::new();
    let launch = f.draft("tool-operation", Some(action()));
    f.approve(&launch);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let input = ToolPermissionInput {
        id: id(),
        title: "Write the release notes".into(),
        details: serde_json::json!({"path":"/private/release.md","content":"Exact new content"}),
    };
    assert!(
        f.store
            .request_permission(&f.b, &launch.id, &work.lease, input.clone())
            .is_err()
    );
    let permission = f
        .store
        .request_permission(&f.bw, &launch.id, &work.lease, input.clone())
        .unwrap();
    assert!(permission.decision.is_none());
    assert_eq!(
        f.store
            .request_permission(&f.bw, &launch.id, &work.lease, input.clone())
            .unwrap()
            .digest,
        permission.digest
    );
    let mut changed = input.clone();
    changed.details = serde_json::json!({"path":"/another/file"});
    assert!(
        f.store
            .request_permission(&f.bw, &launch.id, &work.lease, changed)
            .is_err()
    );
    assert_eq!(
        f.store
            .get_task(&f.b, &launch.id)
            .unwrap()
            .permissions
            .len(),
        1
    );
    let public = f.store.get_task(&f.a, &launch.id).unwrap();
    assert!(public.permissions.is_empty());
    assert!(
        !serde_json::to_string(&public)
            .unwrap()
            .contains("Exact new content")
    );
    assert!(
        f.store
            .decide_permission(
                &f.bw,
                &launch.id,
                &input.id,
                &permission.digest,
                Decision::Approve
            )
            .is_err()
    );
    assert!(
        f.store
            .decide_permission(
                &f.a,
                &launch.id,
                &input.id,
                &permission.digest,
                Decision::Approve
            )
            .is_err()
    );
    assert!(
        f.store
            .decide_permission(&f.b, &launch.id, &input.id, "changed", Decision::Approve)
            .is_err()
    );
    assert!(
        f.store
            .stage_result(
                &f.bw,
                &launch.id,
                &work.lease,
                DraftInput {
                    text: "done".into(),
                    ..Default::default()
                }
            )
            .is_err()
    );
    f.store
        .decide_permission(
            &f.b,
            &launch.id,
            &input.id,
            &permission.digest,
            Decision::Approve,
        )
        .unwrap();
    assert_eq!(
        f.store
            .permission_status(&f.bw, &launch.id, &work.lease, &input.id)
            .unwrap()
            .decision,
        Some(Decision::Approve)
    );
    assert!(
        f.store
            .decide_permission(
                &f.b,
                &launch.id,
                &input.id,
                &permission.digest,
                Decision::Decline
            )
            .is_err()
    );
    let other = worker(&f.store, &f.b, &f.agent);
    assert!(
        f.store
            .permission_status(&other, &launch.id, &work.lease, &input.id)
            .is_err()
    );
    f.store.cancel(&f.b, &launch.id).unwrap();
    assert!(
        f.store
            .permission_status(&f.bw, &launch.id, &work.lease, &input.id)
            .is_err()
    );
    assert!(
        f.store
            .decide_permission(
                &f.b,
                &launch.id,
                &input.id,
                &permission.digest,
                Decision::Approve
            )
            .is_err()
    );
}

#[test]
fn tool_permission_expires_with_the_agent_lease() {
    let f = Fixture::new();
    let launch = f.draft("tool-expiry", Some(action()));
    f.approve(&launch);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let permission = f
        .store
        .request_permission(
            &f.bw,
            &launch.id,
            &work.lease,
            ToolPermissionInput {
                id: id(),
                title: "Run tests".into(),
                details: serde_json::json!({"command":"cargo test"}),
            },
        )
        .unwrap();
    f.advance(LEASE_TTL + 1);
    assert!(
        f.store
            .decide_permission(
                &f.b,
                &launch.id,
                &permission.id,
                &permission.digest,
                Decision::Approve
            )
            .is_err()
    );
    assert!(
        f.store
            .permission_status(&f.bw, &launch.id, &work.lease, &permission.id)
            .is_err()
    );
    assert_eq!(
        f.store.get_task(&f.b, &launch.id).unwrap().status,
        Status::Failed
    );
}

#[test]
fn slack_linking_requires_the_original_human_session_and_single_use_exchange() {
    use crate::slack::{Slack, SlackDestination};
    let f = Fixture::new();
    assert!(f.store.begin_slack(&f.bw).is_err());
    let start = f.store.begin_slack(&f.b).unwrap();
    let provider = Slack::new("app".into(), "secret".into());
    let url = url::Url::parse(&provider.authorize_url("https://litecom.example", &start)).unwrap();
    let pairs = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(pairs["scope"], "chat:write,im:write");
    assert!(!pairs.contains_key("code_challenge"));
    f.store.take_slack_link(&start.state).unwrap();
    assert!(f.store.take_slack_link(&start.state).is_err());
    let destination = SlackDestination {
        token: "fake-token".into(),
        refresh_token: None,
        expires_at: None,
        user: "U1".into(),
        team: "T1".into(),
        channel: "D1".into(),
    };
    f.store.stage_slack_link(&start.state, destination).unwrap();
    assert!(f.store.finish_slack_link(&f.a, &start.state).is_err());
    assert!(f.store.finish_slack_link(&f.bw, &start.state).is_err());
    let mut other_browser = f.b.clone();
    other_browser.credential_id = "another-browser".into();
    assert!(
        f.store
            .finish_slack_link(&other_browser, &start.state)
            .is_err()
    );
    f.store.finish_slack_link(&f.b, &start.state).unwrap();
    f.store.finish_slack_link(&f.b, &start.state).unwrap();
    assert_eq!(f.store.channels(&f.b).unwrap()[0].kind, "slack");
    assert!(
        !serde_json::to_string(&f.store.channels(&f.b).unwrap())
            .unwrap()
            .contains("fake-token")
    );
    let start = f.store.begin_slack(&f.b).unwrap();
    f.advance(601);
    assert!(f.store.take_slack_link(&start.state).is_err());
}

#[test]
fn retrying_slack_completion_does_not_reenable_or_replace_a_channel() {
    use crate::slack::SlackDestination;
    let f = Fixture::new();
    let start = f.store.begin_slack(&f.b).unwrap();
    f.store.take_slack_link(&start.state).unwrap();
    f.store
        .stage_slack_link(
            &start.state,
            SlackDestination {
                token: "first-token".into(),
                refresh_token: None,
                expires_at: None,
                user: "U1".into(),
                team: "T1".into(),
                channel: "D1".into(),
            },
        )
        .unwrap();
    f.store.finish_slack_link(&f.b, &start.state).unwrap();
    // The first response can be lost after its transaction commits.
    f.store.finish_slack_link(&f.b, &start.state).unwrap();
    let channel = f.store.channels(&f.b).unwrap().remove(0);
    f.store.disable_channel(&f.b, &channel.id).unwrap();
    f.store.finish_slack_link(&f.b, &start.state).unwrap();
    assert!(!f.store.channels(&f.b).unwrap()[0].verified);
    f.store
        .verified_channel(&f.b, "slack", "new destination")
        .unwrap();
    f.store.finish_slack_link(&f.b, &start.state).unwrap();
    let stored: Vec<u8> = f
        .store
        .db
        .query_row(
            "SELECT destination FROM channels WHERE id=?",
            [&channel.id],
            |r| r.get(0),
        )
        .unwrap();
    let destination: String = f
        .store
        .vault
        .open(&stored, &format!("channel:{}", channel.id))
        .unwrap();
    assert_eq!(destination, "new destination");
    assert!(f.store.finish_slack_link(&f.a, &start.state).is_err());
    assert!(f.store.take_slack_link(&start.state).is_err());
    f.advance(601);
    assert!(f.store.finish_slack_link(&f.b, &start.state).is_err());
}

#[tokio::test]
async fn slack_transport_exchanges_refreshes_and_updates_the_original_message() {
    use axum::{
        Json, Router,
        body::{Body, to_bytes},
        extract::{Path, State},
        http::Request,
        routing::post,
    };
    use serde_json::{Value, json};
    use std::sync::Mutex;
    type Captured = Arc<Mutex<Vec<(String, Value, String)>>>;
    async fn endpoint(
        State(captured): State<Captured>,
        Path(method): Path<String>,
        request: Request<Body>,
    ) -> Json<Value> {
        let authorization = request
            .headers()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let is_json = request
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("application/json");
        let bytes = to_bytes(request.into_body(), 1_000_000).await.unwrap();
        let payload: Value = if is_json {
            serde_json::from_slice(&bytes).unwrap()
        } else {
            Value::Object(
                url::form_urlencoded::parse(&bytes)
                    .map(|(k, v)| (k.into_owned(), json!(v)))
                    .collect(),
            )
        };
        captured
            .lock()
            .unwrap()
            .push((method.clone(), payload.clone(), authorization));
        Json(match method.as_str() {
            "oauth.v2.access" if payload["grant_type"] == "refresh_token" => {
                json!({"ok":true,"access_token":"new-token","refresh_token":"new-refresh","expires_in":43200})
            }
            "oauth.v2.access" => {
                json!({"ok":true,"access_token":"old-token","token_type":"bot","refresh_token":"old-refresh","expires_in":1,"authed_user":{"id":"U1"},"team":{"id":"T1"}})
            }
            "conversations.open" => json!({"ok":true,"channel":{"id":"D1"}}),
            "chat.postMessage" | "chat.update" => json!({"ok":true,"ts":"123.456"}),
            _ => panic!("Unexpected Slack method"),
        })
    }
    let captured: Captured = Arc::new(Mutex::new(vec![]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/{method}", post(endpoint))
        .with_state(captured.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let slack = crate::slack::Slack::test_api("app".into(), "secret".into(), base);
    let http = reqwest::Client::new();
    let destination = slack
        .exchange(&http, "https://litecom.example", "code")
        .await
        .unwrap();
    let destination = slack.refresh(&http, &destination).await.unwrap();
    let remote = slack
        .notify(&http, &destination, "Ready for review", None)
        .await
        .unwrap();
    let updated = slack
        .notify(&http, &destination, "Approved", Some(&remote))
        .await
        .unwrap();
    assert_eq!(remote, updated);
    let calls = captured.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[0].1["code"], "code");
    assert!(calls[0].2.starts_with("Basic "));
    assert_eq!(calls[1].1["users"], "U1");
    assert_eq!(calls[2].1["refresh_token"], "old-refresh");
    assert_eq!(calls[3].0, "chat.postMessage");
    assert_eq!(calls[3].2, "Bearer new-token");
    assert_eq!(calls[4].0, "chat.update");
    assert_eq!(calls[4].1["ts"], "123.456");
    assert_eq!(calls[4].1["text"], "Approved");
    server.abort();
}

#[test]
fn named_acp_agents_use_native_commands_and_supported_mcp_paths() {
    use crate::runner::{NativeAgent, RunnerConfig};
    for agent in [
        NativeAgent::OpenCode,
        NativeAgent::Cursor,
        NativeAgent::Pi,
        NativeAgent::OpenClaw,
        NativeAgent::Hermes,
    ] {
        let mut settings = RunnerConfig {
            agent,
            executable: std::env::current_exe().unwrap(),
            workspace: std::env::current_dir().unwrap(),
            arguments: agent.default_arguments(),
            mcp: None,
        };
        assert!(agent.is_acp());
        assert_eq!(
            settings.arguments,
            if agent == NativeAgent::Pi {
                vec![]
            } else {
                vec!["acp"]
            }
        );
        settings.validate().unwrap();
        settings.mcp = Some(McpConnection {
            executable: settings.executable.to_string_lossy().into(),
            home: settings.workspace.to_string_lossy().into(),
            connection: "test".into(),
        });
        assert_eq!(
            settings.validate().is_ok(),
            matches!(agent, NativeAgent::OpenCode | NativeAgent::Hermes)
        );
    }
}
