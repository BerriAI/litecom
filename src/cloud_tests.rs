use super::*;
use crate::{api::AppState, cloud, devin::Devin, providers::Providers};
use axum::{
    Json, Router,
    extract::State,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::sync::Mutex;

#[derive(Default)]
struct ProviderFixture {
    starts: usize,
    stops: usize,
    fail_start: bool,
    fail_stop: bool,
    fail_second_file: bool,
    finished: bool,
    waiting: bool,
    created: Vec<Value>,
}
struct Rig {
    state: AppState,
    a: Principal,
    b: Principal,
    agent: String,
    clock: Arc<AtomicI64>,
    temp: tempfile::TempDir,
    provider: Arc<Mutex<ProviderFixture>>,
    devin: Devin,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn endpoint(
    State(f): State<Arc<Mutex<ProviderFixture>>>,
    request: axum::extract::Request,
) -> Response {
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer cog_fixture"
    );
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let body = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let mut f = f.lock().unwrap();
    if path == "/v3/self" {
        return Json(json!({"principal_type":"pat_user","user_id":"user-moe","user_name":"Moe","devin_sessions_org_id":"org-fixture"})).into_response();
    }
    if path.ends_with("/sessions") {
        if method == Method::GET {
            return Json(json!({"items":[]})).into_response();
        }
        assert_eq!(method, Method::POST);
        f.starts += 1;
        f.created.push(serde_json::from_slice(&body).unwrap());
        if f.fail_start {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    } else if path.ends_with("/session-fixture") && method == Method::DELETE {
        f.stops += 1;
        return if f.fail_stop {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NO_CONTENT
        }
        .into_response();
    } else if path.ends_with("/attachments") {
        return Json(json!([
            {"attachment_id":"file-one","name":"notes.txt","content_type":"text/plain","source":"devin"},
            {"attachment_id":"file-two","name":"image.png","content_type":"image/png","source":"devin"},
            {"attachment_id":"not-selected","name":"private.txt","source":"devin"},
            {"attachment_id":"user-file","name":"input.txt","source":"user"}
        ])).into_response();
    } else if path.contains("/attachments/") {
        if path.ends_with("/notes.txt") {
            return b"exact notes\n".to_vec().into_response();
        }
        if path.ends_with("/image.png") {
            return if f.fail_second_file {
                StatusCode::SERVICE_UNAVAILABLE.into_response()
            } else {
                vec![137, 80, 78, 71, 0, 1, 2].into_response()
            };
        }
        panic!("Unselected file requested: {path}");
    }
    Json(json!({"session_id":"session-fixture","url":"https://app.devin.ai/sessions/private-fixture","status":"running","status_detail":if f.finished {"finished"} else if f.waiting {"waiting_for_approval"} else {"working"},"structured_output":if f.finished {json!({"text":"Private cloud reply","files":["file-one","image.png"]})}else{Value::Null}})).into_response()
}
impl Rig {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(1000));
        let c = clock.clone();
        let store = Store::with_clock(
            temp.path().join("relay.db"),
            [7; 32],
            Arc::new(move || c.load(Ordering::SeqCst)),
        )
        .unwrap();
        let (a, _) = user(&store, "Theo", "+14155550301");
        let (b, session) = user(&store, "Moe", "+14155550302");
        store.request_contact(&a, &session.profile.invite).unwrap();
        store.accept_contact(&b, &a.user_id).unwrap();
        let provider = Arc::new(Mutex::new(ProviderFixture::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let devin = Devin::test_api(format!("http://{}", listener.local_addr().unwrap()));
        let router = Router::new()
            .fallback(endpoint)
            .with_state(provider.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let connection = devin.connect("cog_fixture", None).await.unwrap();
        let agent = store.connect_devin(&b, "Devin", connection).unwrap().id;
        store.set_default(&b, &agent).unwrap();
        let state = AppState::new(
            store,
            "https://litecom.test".into(),
            Providers::development().unwrap(),
        );
        Self {
            state,
            a,
            b,
            agent,
            clock,
            temp,
            provider,
            devin,
            server,
        }
    }
    fn advance(&self) {
        self.clock.fetch_add(150, Ordering::SeqCst);
    }
    fn reopen(&self) {
        let c = self.clock.clone();
        *self.state.db().unwrap() = Store::with_clock(
            self.temp.path().join("relay.db"),
            [7; 32],
            Arc::new(move || c.load(Ordering::SeqCst)),
        )
        .unwrap();
    }
    fn request(&self, key: &str) -> TaskView {
        self.state
            .db()
            .unwrap()
            .create_task(
                &self.a,
                NewTask {
                    owner_id: self.b.user_id.clone(),
                    agent_id: None,
                    request: "Find the notes and picture.".into(),
                    idempotency_key: key.into(),
                },
            )
            .unwrap()
    }
    fn owner(&self, id: &str) -> TaskView {
        self.state.db().unwrap().get_task(&self.b, id).unwrap()
    }
    fn approve(&self, id: &str) {
        let task = self.owner(id);
        let d = task.draft.unwrap();
        self.state
            .db()
            .unwrap()
            .decide(&self.b, id, d.version, &d.digest, Decision::Approve)
            .unwrap();
    }
    async fn tick(&self) {
        cloud::tick(&self.state, &self.devin).await.unwrap();
    }
    async fn start(&self, key: &str) -> TaskView {
        let task = self.request(key);
        self.tick().await;
        assert_eq!(self.owner(&task.id).status, Status::AwaitingApproval);
        self.approve(&task.id);
        self.tick().await;
        self.owner(&task.id)
    }
}

#[tokio::test]
async fn devin_persists_session_and_delivers_exact_output_after_one_approval() {
    let r = Rig::new().await;
    r.state.db().unwrap().enable_phone(&r.a).unwrap();
    r.state.db().unwrap().enable_phone(&r.b).unwrap();
    assert_eq!(
        r.provider.lock().unwrap().starts,
        0,
        "Connecting must be read-only"
    );
    let task = r.request("one");
    r.tick().await;
    assert_eq!(
        r.provider.lock().unwrap().starts,
        0,
        "Preparing must not launch"
    );
    let stale = r.owner(&task.id).draft.unwrap();
    let same = r.devin.connect("cog_fixture", None).await.unwrap();
    r.state
        .db()
        .unwrap()
        .connect_devin(&r.b, "Devin", same)
        .unwrap();
    r.approve(&task.id);
    r.tick().await;
    assert_eq!(r.owner(&task.id).status, Status::Executing);
    let requester = r.state.db().unwrap().get_task(&r.a, &task.id).unwrap();
    assert!(
        requester.draft.is_none() && requester.execution.is_none() && requester.response.is_none()
    );
    let owner = r.owner(&task.id);
    assert!(
        owner
            .execution
            .unwrap()
            .url
            .unwrap()
            .contains("private-fixture")
    );
    r.advance();
    r.reopen();
    assert_eq!(
        r.owner(&task.id).status,
        Status::Executing,
        "Known remote sessions survive local lease expiry"
    );
    r.provider.lock().unwrap().waiting = true;
    r.tick().await;
    assert!(
        r.owner(&task.id)
            .execution
            .unwrap()
            .message
            .contains("permission decision")
    );
    let recipients:Vec<String>=r.state.db().unwrap().db.prepare("SELECT c.user_id FROM deliveries d JOIN events e ON e.sequence=d.event_id JOIN channels c ON c.id=d.channel_id WHERE e.kind='cloud_attention'").unwrap().query_map([],|row|row.get(0)).unwrap().collect::<std::result::Result<_,_>>().unwrap();
    assert_eq!(
        recipients,
        vec![r.b.user_id.clone()],
        "Only the owner is notified about private provider permissions"
    );
    r.advance();
    {
        let mut f = r.provider.lock().unwrap();
        f.finished = true;
        f.fail_second_file = true;
    }
    r.tick().await;
    assert_eq!(r.owner(&task.id).status, Status::Executing);
    let before: i64 = r
        .state
        .db()
        .unwrap()
        .db
        .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, 1);
    r.advance();
    r.reopen();
    r.provider.lock().unwrap().fail_second_file = false;
    r.tick().await;
    let staged = r.owner(&task.id);
    assert_eq!(staged.status, Status::Completed);
    let draft = staged.draft.unwrap();
    assert_eq!(draft.text, "Private cloud reply");
    assert_eq!(draft.attachments.len(), 2);
    assert_eq!(draft.version, stale.version + 1);
    {
        let db = r.state.db().unwrap();
        assert!(
            db.decide(
                &r.b,
                &task.id,
                stale.version,
                &stale.digest,
                Decision::Approve
            )
            .is_err()
        );
        assert!(db.download(&r.a, &draft.attachments[0].id).is_ok());
        assert!(db.get_task(&r.a, &task.id).unwrap().response.is_some());
        assert_eq!(
            db.db
                .query_row(
                    "SELECT COUNT(*) FROM approvals WHERE task_id=?",
                    [&task.id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            db.db
                .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2,
            "Retry reuses exact uploaded snapshots"
        );
        assert_eq!(
            db.db
                .query_row("SELECT COUNT(*) FROM cloud_jobs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "Completed job token copy is removed"
        );
    }
    assert_eq!(
        r.state
            .db()
            .unwrap()
            .download(&r.a, &draft.attachments[0].id)
            .unwrap()
            .1,
        b"exact notes\n"
    );
    let f = r.provider.lock().unwrap();
    assert_eq!(f.starts, 1);
    assert!(f.created[0].get("max_acu_limit").is_none());
    assert!(f.created[0].get("max_tokens").is_none());
    assert!(f.created[0].get("bypass_approval").is_none());
    assert!(f.created[0].get("security_profile").is_none());
    assert!(
        f.created[0]["tags"]
            .as_array()
            .unwrap()
            .contains(&json!(task.id))
    );
}

#[tokio::test]
async fn devin_unknown_launch_is_never_repeated_after_restart() {
    let r = Rig::new().await;
    r.provider.lock().unwrap().fail_start = true;
    let task = r.start("unknown").await;
    assert_eq!(task.status, Status::Failed);
    assert_eq!(task.receipt.unwrap().outcome, Outcome::Uncertain);
    for _ in 0..3 {
        r.advance();
        r.reopen();
        r.tick().await;
    }
    assert_eq!(r.provider.lock().unwrap().starts, 1);
}

#[tokio::test]
async fn devin_crash_during_start_reservation_is_uncertain_and_cannot_be_reclaimed_to_launch() {
    let r = Rig::new().await;
    let task = r.request("crash");
    r.tick().await;
    r.approve(&task.id);
    r.state.db().unwrap().queue_cloud_work().unwrap();
    let mut old = r.state.db().unwrap().claim_cloud_job().unwrap().unwrap();
    assert!(r.state.db().unwrap().claim_cloud_job().unwrap().is_none());
    r.advance();
    r.reopen();
    r.tick().await;
    assert_eq!(
        r.owner(&task.id).receipt.unwrap().outcome,
        Outcome::Uncertain
    );
    let session = crate::devin::Session {
        session_id: "late".into(),
        url: String::new(),
        status: "running".into(),
        status_detail: None,
        structured_output: None,
    };
    assert!(
        r.state
            .db()
            .unwrap()
            .cloud_started(&mut old, session)
            .is_err()
    );
    assert_eq!(r.provider.lock().unwrap().starts, 0);
}

#[tokio::test]
async fn devin_disconnect_waits_for_confirmed_remote_termination() {
    let r = Rig::new().await;
    let task = r.start("cancel").await;
    r.provider.lock().unwrap().fail_stop = true;
    r.state.db().unwrap().revoke_agent(&r.b, &r.agent).unwrap();
    r.advance();
    r.tick().await;
    assert_eq!(r.owner(&task.id).status, Status::CancelRequested);
    assert!(
        r.owner(&task.id)
            .execution
            .unwrap()
            .message
            .contains("not confirmed")
    );
    r.reopen();
    r.advance();
    r.provider.lock().unwrap().fail_stop = false;
    r.tick().await;
    assert_eq!(r.owner(&task.id).status, Status::Cancelled);
    assert_eq!(r.provider.lock().unwrap().stops, 2);
    assert_eq!(r.provider.lock().unwrap().starts, 1);
    let db = r.state.db().unwrap();
    assert!(
        db.db
            .query_row(
                "SELECT config IS NULL FROM agents WHERE id=?",
                [&r.agent],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[tokio::test]
async fn devin_credential_changes_invalidate_launch_and_external_workers_cannot_claim_cloud_work() {
    let r = Rig::new().await;
    let task = r.request("changed");
    r.tick().await;
    r.approve(&task.id);
    let p = worker(&r.state.db().unwrap(), &r.b, &r.agent);
    assert!(r.state.db().unwrap().claim(&p, &r.agent).is_err());
    let mut changed = r.devin.connect("cog_fixture", None).await.unwrap();
    changed.token = "cog_changed".into();
    assert!(
        r.state
            .db()
            .unwrap()
            .connect_devin(&p, "other", changed.clone())
            .is_err()
    );
    r.state
        .db()
        .unwrap()
        .connect_devin(&r.b, "Devin", changed)
        .unwrap();
    r.tick().await;
    assert_eq!(r.owner(&task.id).status, Status::Failed);
    assert_eq!(r.provider.lock().unwrap().starts, 0);
    let json =
        serde_json::to_string(&r.state.db().unwrap().agents(&r.a, &r.b.user_id).unwrap()).unwrap();
    assert!(!json.contains("cog_") && !json.contains("org-fixture"));
}
