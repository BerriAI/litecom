use super::*;
use crate::{
    api::{AppState, router},
    error::Error,
    providers::Providers,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

pub(super) fn allow_member(store: &Store, user: &str) {
    let key = hash(user);
    let identity = crate::google::Identity {
        subject: user.into(),
        email: format!("{user}@example.com"),
        name: "Member".into(),
        audience: "test-client".into(),
        domain: Some("example.com".into()),
    };
    store.db.execute("INSERT INTO login_identities(provider,subject_key,user_id,data) VALUES('google',?,?,?)",
        rusqlite::params![key,user,store.vault.seal(&identity,&format!("google:{key}")).unwrap()]).unwrap();
}

#[test]
fn workspace_policy_stops_queued_and_running_work_from_removed_members() {
    let mut f = Fixture::new();
    let task = f.draft("run", Some(action()));
    f.approve(&task);
    let running = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let queued = f.request("queued");
    allow_member(&f.store, &f.b.user_id);
    f.store.workspace_policy = Some(crate::google::Google::new(
        "test-client".into(),
        "secret".into(),
        "example.com",
    ));
    assert!(matches!(
        f.store.heartbeat(&f.bw, &task.id, &running.lease),
        Err(Error::Forbidden(_))
    ));
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    assert_eq!(
        f.store.get_task(&f.b, &queued.id).unwrap().status,
        Status::Cancelled
    );
    assert_eq!(
        f.store.get_task(&f.b, &task.id).unwrap().status,
        Status::CancelRequested
    );
    // Rejoining must not resurrect cancelled or uncertain work.
    allow_member(&f.store, &f.a.user_id);
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
}

#[test]
fn task_limits_survive_restart_keep_retries_and_do_not_block_other_members() {
    let mut f = Fixture::new();
    f.store.limits.requests_per_minute = 1;
    let task = f.request("one");
    assert_eq!(f.request("one").id, task.id);
    let input = |key: &str| NewTask {
        ask_only: false,
        owner_id: f.b.user_id.clone(),
        agent_id: None,
        request: "Work".into(),
        idempotency_key: key.into(),
    };
    assert!(matches!(
        f.store.create_task(&f.a, input("two")),
        Err(Error::RateLimited)
    ));
    f.store.create_task(&f.b, input("other-user")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    f.store.db.backup(rusqlite::MAIN_DB, &path, None).unwrap();
    let mut reopened = Store::with_clock(path, [7; 32], Arc::new(|| 1000)).unwrap();
    reopened.limits.requests_per_minute = 1;
    assert!(matches!(
        reopened.create_task(&f.a, input("two")),
        Err(Error::RateLimited)
    ));
    f.advance(61);
    f.store.limits.pending_requests = 1;
    assert!(matches!(
        f.store.create_task(&f.a, input("two")),
        Err(Error::Conflict(_))
    ));
    f.store.cancel(&f.a, &task.id).unwrap();
    f.store.create_task(&f.a, input("two")).unwrap();
}

#[test]
fn uploads_are_bounded_atomic_and_release_capacity_after_failure() {
    let mut f = Fixture::new();
    f.store.limits.file_bytes = 10;
    f.store.limits.task_file_bytes = 12;
    f.store.limits.stored_file_bytes = 15;
    let task = f.request("files");
    let mut first = f
        .store
        .begin_upload(&f.b, &task.id, None, "a.txt", "text/plain")
        .unwrap();
    let second = f
        .store
        .begin_upload(&f.b, &task.id, None, "b.txt", "text/plain")
        .unwrap();
    assert!(matches!(
        f.store
            .begin_upload(&f.b, &task.id, None, "c.txt", "text/plain"),
        Err(Error::RateLimited)
    ));
    assert!(first.write(b"01234567890").is_err());
    drop(first);
    drop(second);
    let one = f
        .store
        .upload(&f.b, &task.id, None, "a.txt", "text/plain", b"12345678")
        .unwrap();
    assert_eq!(
        f.store
            .upload(&f.b, &task.id, None, "a.txt", "text/plain", b"12345678")
            .unwrap()
            .id,
        one.id
    );
    assert!(
        f.store
            .upload(&f.b, &task.id, None, "b.txt", "text/plain", b"12345")
            .is_err()
    );
    let other = f.request("other-files");
    assert!(
        f.store
            .upload(&f.b, &other.id, None, "a.txt", "text/plain", b"12345678")
            .is_err()
    );
    assert_eq!(
        f.store
            .db
            .query_row("SELECT COUNT(*) FROM artifacts", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    f.store
        .upload(&f.b, &other.id, None, "a.txt", "text/plain", b"123")
        .unwrap();
}

#[tokio::test]
async fn oversized_json_is_rejected_and_revoked_credentials_cannot_fetch_files() {
    let f = Fixture::new();
    let (_, session) = user(&f.store, "Guest", "+14155550103");
    let task = f.request("private-file");
    let file = f
        .store
        .upload(
            &f.b,
            &task.id,
            None,
            "private.txt",
            "text/plain",
            b"private bytes",
        )
        .unwrap();
    let device = f.store.begin_device("download").unwrap();
    f.store
        .authorize_device(&f.b, &device.user_code, &f.agent)
        .unwrap();
    let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
    let principal = f.store.authenticate(&token.token, Role::Agent).unwrap();
    let state = AppState::new(
        f.store,
        "http://localhost".into(),
        Providers::development().unwrap(),
    );
    let app = router(state.clone());
    let oversized = app
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token.token))
                .body(Body::from(" ".repeat(2 * 1024 * 1024 + 1)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let stranger = app
        .clone()
        .oneshot(
            Request::get(format!("/api/files/{}", file.id))
                .header("cookie", format!("litecom_session={}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stranger.status(), StatusCode::FORBIDDEN);
    state
        .db()
        .unwrap()
        .revoke_connection(&f.b, &principal.credential_id)
        .unwrap();
    let revoked = app
        .oneshot(
            Request::get(format!("/api/files/{}", file.id))
                .header("authorization", format!("Bearer {}", token.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn reopening_the_database_preserves_valid_browser_and_agent_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    let store = Store::open(&path, [9; 32]).unwrap();
    let (p, session) = user(&store, "Member", "+14155550104");
    let agent = store.agents(&p, &p.user_id).unwrap()[0].id.clone();
    let code = store.begin_device("existing").unwrap();
    store.authorize_device(&p, &code.user_code, &agent).unwrap();
    let device = store.poll_device(&code.device_code).unwrap().unwrap();
    drop(store);
    let store = Store::open(&path, [9; 32]).unwrap();
    assert_eq!(
        store
            .authenticate(&session.token, Role::Human)
            .unwrap()
            .user_id,
        p.user_id
    );
    assert_eq!(
        store
            .authenticate(&device.token, Role::Agent)
            .unwrap()
            .agent_id,
        Some(agent)
    );
}

#[test]
fn upgrades_keep_live_devices_connected_without_restoring_expired_or_revoked_credentials() {
    let f = Fixture::new();
    let (_, browser) = user(&f.store, "Browser", "+14155550105");
    let mut devices = Vec::new();
    for label in ["live", "expired", "revoked"] {
        let code = f.store.begin_device(label).unwrap();
        f.store
            .authorize_device(&f.b, &code.user_code, &f.agent)
            .unwrap();
        let device = f.store.poll_device(&code.device_code).unwrap().unwrap();
        assert_eq!(device.expires_at, i64::MAX);
        // Model credentials issued by the previous release.
        f.store
            .db
            .execute(
                "UPDATE credentials SET expires_at=?,revoked=? WHERE token_hash=?",
                rusqlite::params![
                    if label == "expired" { 999 } else { 2000 },
                    label == "revoked",
                    hash(&device.token)
                ],
            )
            .unwrap();
        devices.push(device);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.db");
    f.store.db.backup(rusqlite::MAIN_DB, &path, None).unwrap();
    // Migration runs while the original live credential is still valid.
    let clock = f.clock.clone();
    let store = Store::with_clock(
        path,
        [7; 32],
        Arc::new(move || clock.load(std::sync::atomic::Ordering::SeqCst)),
    )
    .unwrap();
    f.advance(400 * 24 * 60 * 60);
    assert!(store.authenticate(&devices[0].token, Role::Agent).is_ok());
    for device in &devices[1..] {
        assert!(matches!(
            store.authenticate(&device.token, Role::Agent),
            Err(Error::Unauthorized)
        ));
    }
    assert!(matches!(
        store.authenticate(&browser.token, Role::Human),
        Err(Error::Unauthorized)
    ));
    let principal = store.authenticate(&devices[0].token, Role::Agent).unwrap();
    store
        .revoke_connection(&f.b, &principal.credential_id)
        .unwrap();
    assert!(matches!(
        store.authenticate(&devices[0].token, Role::Agent),
        Err(Error::Unauthorized)
    ));
}

fn read_harness(request: &str) -> Action {
    Action::Harness {
        title: "Run Codex".into(),
        description: "Fixture".into(),
        runner: "codex".into(),
        executable: "/fixture/codex".into(),
        arguments: vec![],
        mcp: None,
        instruction: request.into(),
        workspace: "/fixture".into(),
        permissions: vec!["Fixture".into()],
        acting_account: "Owner".into(),
        share_result: true,
    }
}

#[test]
fn read_only_approval_cannot_be_upgraded_or_claimed_by_old_workers() {
    for ask_only in [false, true] {
        let f = Fixture::new();
        let task = f
            .store
            .create_task(
                &f.a,
                NewTask {
                    owner_id: f.b.user_id.clone(),
                    agent_id: None,
                    request: "Read a file".into(),
                    idempotency_key: id(),
                    ask_only,
                },
            )
            .unwrap();
        let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
        let prepared = f
            .store
            .prepare(
                &f.bw,
                &task.id,
                Some(&work.lease),
                DraftInput {
                    text: String::new(),
                    attachment_ids: vec![],
                    action: Some(read_harness(&task.request)),
                },
            )
            .unwrap();
        let draft = prepared.draft.unwrap();
        // Ask-only applies even when an older approval client sends plain Approve.
        let approved = f
            .store
            .decide_access(
                &f.b,
                &task.id,
                draft.version,
                &draft.digest,
                Decision::Approve,
                !ask_only,
            )
            .unwrap();
        assert_eq!(approved.approved_access, Some(RunAccess::ReadOnly));
        assert!(approved.draft.unwrap().action.unwrap().read_only());
        assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
        assert_eq!(
            f.store.get_task(&f.b, &task.id).unwrap().status,
            Status::Approved
        );
        // Replaying an action approval never widens the existing decision.
        let replay = f
            .store
            .decide(
                &f.b,
                &task.id,
                draft.version,
                &draft.digest,
                Decision::Approve,
            )
            .unwrap();
        assert_eq!(replay.approved_access, Some(RunAccess::ReadOnly));
        let work = f
            .store
            .claim_read_capable(&f.bw, &f.agent)
            .unwrap()
            .unwrap();
        f.store
            .check_read_lease(&f.bw, &task.id, &work.lease)
            .unwrap();
        assert!(
            f.store
                .check_read_lease(&f.a, &task.id, &work.lease)
                .is_err()
        );
        assert!(
            f.store
                .request_permission(
                    &f.bw,
                    &task.id,
                    &work.lease,
                    ToolPermissionInput {
                        id: id(),
                        title: "Write a file".into(),
                        details: serde_json::json!({})
                    }
                )
                .is_err()
        );
        f.store.cancel(&f.b, &task.id).unwrap();
        assert!(
            f.store
                .check_read_lease(&f.bw, &task.id, &work.lease)
                .is_err()
        );
    }
}

#[test]
fn read_only_request_retries_cannot_switch_to_actions() {
    let f = Fixture::new();
    let mut input = NewTask {
        owner_id: f.b.user_id.clone(),
        agent_id: None,
        request: "Read".into(),
        idempotency_key: id(),
        ask_only: true,
    };
    let task = f.store.create_task(&f.a, input.clone()).unwrap();
    assert_eq!(
        f.store.create_task(&f.a, input.clone()).unwrap().id,
        task.id
    );
    input.ask_only = false;
    assert!(f.store.create_task(&f.a, input).is_err());
}

#[tokio::test]
async fn slack_reads_require_a_live_read_only_lease_and_owner_membership() {
    use axum::{
        Json,
        extract::{Path as AxumPath, Query, State},
        routing::get,
    };
    use serde_json::{Value, json};
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    async fn slack(
        State(calls): State<Arc<std::sync::Mutex<Vec<String>>>>,
        AxumPath(method): AxumPath<String>,
        Query(q): Query<std::collections::HashMap<String, String>>,
    ) -> Json<Value> {
        calls.lock().unwrap().push(method.clone());
        Json(match method.as_str() {
            "conversations.members" => {
                json!({"ok":true,"members":if q.get("channel").map(String::as_str)==Some("COWN") {vec!["UOWNER"]} else {vec!["UOTHER"]},"response_metadata":{"next_cursor":""}})
            }
            "conversations.history" | "conversations.replies" => {
                json!({"ok":true,"messages":[{"user":"UOWNER","text":"Authorized fixture","ts":"1.1","private_metadata":"omit"}],"response_metadata":{"next_cursor":""}})
            }
            "users.conversations" => {
                assert_eq!(q.get("user").map(String::as_str), Some("UOWNER"));
                json!({"ok":true,"channels":[{"id":"COWN","name":"allowed"}],"response_metadata":{"next_cursor":""}})
            }
            _ => panic!("Unexpected Slack operation: {method}"),
        })
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(
        axum::serve(
            listener,
            axum::Router::new()
                .route("/{method}", get(slack))
                .with_state(calls.clone()),
        )
        .into_future(),
    );
    let f = Fixture::new();
    let destination = crate::slack::SlackDestination {
        token: "fixture-token".into(),
        refresh_token: None,
        expires_at: None,
        user: "UOWNER".into(),
        team: "TFIXTURE".into(),
        channel: "DOWNER".into(),
    };
    f.store
        .verified_channel(&f.b, "slack", &serde_json::to_string(&destination).unwrap())
        .unwrap();
    let code = f.store.begin_device("Read test").unwrap();
    f.store
        .authorize_device(&f.b, &code.user_code, &f.agent)
        .unwrap();
    let token = f.store.poll_device(&code.device_code).unwrap().unwrap();
    let worker = f.store.authenticate(&token.token, Role::Agent).unwrap();
    let task = f.draft("slack-read", Some(read_harness("Read Slack")));
    let draft = task.draft.unwrap();
    f.store
        .decide_access(
            &f.b,
            &task.id,
            draft.version,
            &draft.digest,
            Decision::Approve,
            true,
        )
        .unwrap();
    let work = f
        .store
        .claim_read_capable(&worker, &f.agent)
        .unwrap()
        .unwrap();
    let mut providers = Providers::development().unwrap();
    providers.slack = Some(crate::slack::Slack::test_api(
        "id".into(),
        "secret".into(),
        base,
    ));
    let state = AppState::new(f.store, "http://localhost".into(), providers);
    let app = router(state.clone());
    let request = |resource: &str, channel: &str, lease: &str| {
        Request::post(format!("/api/tasks/{}/read/slack", task.id))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token.token))
            .body(Body::from(
                json!({"lease":lease,"read":{"resource":resource,"channel":channel}}).to_string(),
            ))
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(request("history", "COWN", &work.lease))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["messages"][0]["text"], "Authorized fixture");
    assert!(result["messages"][0].get("private_metadata").is_none());
    let denied = app
        .clone()
        .oneshot(request("history", "COTHER", &work.lease))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.as_str() == "conversations.history")
            .count(),
        1
    );
    assert_eq!(
        app.clone()
            .oneshot(request("post_message", "COWN", &work.lease))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.clone()
            .oneshot(request("history", "COWN", "forged"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    state.db().unwrap().cancel(&f.b, &task.id).unwrap();
    let count = calls.lock().unwrap().len();
    assert_eq!(
        app.oneshot(request("history", "COWN", &work.lease))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(calls.lock().unwrap().len(), count);
    server.abort();
}
