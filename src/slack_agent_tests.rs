use super::*;
use crate::{
    api::{AppState, router},
    providers::Providers,
    slack_agent::{Job, Mention, SlackAgent, step},
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, Request, StatusCode},
    routing::any,
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct Mock {
    models: Arc<Mutex<VecDeque<Value>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    thread: Arc<Mutex<Vec<Value>>>,
    fail_post: Arc<std::sync::atomic::AtomicBool>,
}
async fn mock(
    State(m): State<Mock>,
    OriginalUri(uri): OriginalUri,
    _headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let value: Value = if body.is_empty() {
        json!(
            url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .into_owned()
                .collect::<std::collections::HashMap<_, _>>()
        )
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    m.calls
        .lock()
        .unwrap()
        .push((uri.path().into(), value.clone()));
    let response = match uri.path() {
        "/v1/chat/completions" => {
            let message = m
                .models
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call");
            json!({"choices":[{"finish_reason":if message["tool_calls"].is_array(){"tool_calls"}else{"stop"},"message":message}]})
        }
        "/chat.getPermalink" => {
            json!({"ok":true,"permalink":"https://example.slack.com/archives/C1/p1000000001"})
        }
        "/conversations.replies" | "/conversations.history" => {
            json!({"ok":true,"messages":*m.thread.lock().unwrap(),"response_metadata":{"next_cursor":""}})
        }
        "/chat.postMessage" | "/chat.update" => {
            if m.fail_post.swap(false, Ordering::SeqCst) {
                return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"ok":false})));
            }
            json!({"ok":true,"ts":value["ts"].as_str().unwrap_or("1001.000001")})
        }
        other => panic!("unexpected Slack API {other}"),
    };
    (StatusCode::OK, Json(response))
}
fn tool(name: &str, args: Value) -> Value {
    json!({"role":"assistant","content":null,"tool_calls":[{"id":id(),"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
}
fn mention(event: &str) -> Mention {
    Mention {
        team: "T1".into(),
        event_id: event.into(),
        user: "U1".into(),
        channel: "C1".into(),
        ts: "1000.000001".into(),
        thread_ts: "999.000001".into(),
        text: "@Litecom can you get this from Moe?".into(),
    }
}
fn link(store: &Store, p: &Principal, user: &str, team: &str) {
    store
        .verified_channel(
            p,
            "slack",
            &serde_json::to_string(&crate::slack::SlackDestination {
                token: "private-slack-token".into(),
                refresh_token: None,
                expires_at: None,
                user: user.into(),
                team: team.into(),
                channel: format!("D{user}"),
            })
            .unwrap(),
        )
        .unwrap();
}
struct Test {
    state: AppState,
    mock: Mock,
    server: tokio::task::JoinHandle<()>,
    owner: Principal,
    requester: Principal,
    worker: Principal,
    agent: String,
    clock: Arc<AtomicI64>,
}
impl Drop for Test {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Test {
    async fn new() -> Self {
        let f = Fixture::new();
        link(&f.store, &f.a, "U1", "T1");
        link(&f.store, &f.b, "U2", "T1");
        let mock = Mock::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/{*path}", any(self::mock))
            .with_state(mock.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut providers = Providers::test_replies(&base, "signing-secret");
        providers.slack = Some(crate::slack::Slack::test_api(
            "client".into(),
            "secret".into(),
            base.clone(),
        ));
        providers.slack_agent = SlackAgent::from_env(&|key| match key {
            "SLACK_AGENT_MODEL" => Some("test-model".into()),
            "SLACK_AGENT_BASE_URL" => Some(format!("{base}/v1")),
            "SLACK_AGENT_API_KEY" => Some("model-secret".into()),
            _ => None,
        })
        .unwrap();
        Self {
            state: AppState::new(f.store, "https://litecom.example".into(), providers),
            mock,
            server,
            owner: f.b,
            requester: f.a,
            worker: f.bw,
            agent: f.agent,
            clock: f.clock,
        }
    }
    async fn tick(&self) -> Job {
        let mut job = self
            .state
            .db()
            .unwrap()
            .claim_slack()
            .unwrap()
            .expect("job ready");
        let (done, delay) = step(&self.state, &mut job).await.unwrap();
        self.state
            .db()
            .unwrap()
            .save_slack(&job, done, delay)
            .unwrap();
        job
    }
    fn push(&self, messages: Vec<Value>) {
        self.mock.models.lock().unwrap().extend(messages);
    }
    fn count(&self, table: &str) -> i64 {
        self.state
            .db()
            .unwrap()
            .db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

fn signed(body: Value, valid: bool) -> Request<Body> {
    let text = body.to_string();
    let now = crate::store::now();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(if valid {
        b"signing-secret"
    } else {
        b"wrong-secret"
    })
    .unwrap();
    mac.update(format!("v0:{now}:{text}").as_bytes());
    Request::builder()
        .method("POST")
        .uri("/webhooks/slack/events")
        .header("content-type", "application/json")
        .header("x-slack-request-timestamp", now.to_string())
        .header(
            "x-slack-signature",
            format!("v0={}", hex::encode(mac.finalize().into_bytes())),
        )
        .body(Body::from(text))
        .unwrap()
}
fn event() -> Value {
    json!({"type":"event_callback","team_id":"T1","event_id":"Ev1","event":{"type":"app_mention","user":"U1","channel":"C1","ts":"1000.000001","text":"ask Moe about this"}})
}

#[tokio::test]
async fn generated_manifest_keeps_approvals_and_adds_mentions_only_when_enabled() {
    use http_body_util::BodyExt;
    let t = Test::new().await;
    let mut state = t.state.clone();
    state.providers.slack_agent = None;
    for (path, enabled) in [
        ("/setup/slack/manifest", false),
        ("/setup/slack/manifest?agent=1", true),
    ] {
        let response = router(state.clone())
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let manifest: Value = serde_json::from_slice(&body).unwrap();
        let scopes = manifest["oauth_config"]["scopes"]["bot"]
            .as_array()
            .unwrap();
        assert!(scopes.contains(&json!("chat:write")) && scopes.contains(&json!("im:write")));
        assert_eq!(scopes.contains(&json!("app_mentions:read")), enabled);
        assert_eq!(
            manifest["settings"]["interactivity"]["request_url"],
            "https://litecom.example/webhooks/slack"
        );
        if enabled {
            assert_eq!(
                manifest["settings"]["event_subscriptions"]["request_url"],
                "https://litecom.example/webhooks/slack/events"
            );
        }
        assert!(!String::from_utf8_lossy(&body).contains("secret"));
    }
}

#[tokio::test]
async fn signed_events_are_durable_deduplicated_and_optional() {
    let t = Test::new().await;
    let app = router(t.state.clone());
    assert_eq!(
        app.clone()
            .oneshot(signed(event(), false))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(t.count("slack_jobs"), 0);
    for _ in 0..2 {
        assert_eq!(
            app.clone()
                .oneshot(signed(event(), true))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(t.count("slack_jobs"), 1);
    assert_eq!(t.count("tasks"), 0);
    assert!(t.mock.calls.lock().unwrap().is_empty()); // webhook never waits on a model
    let mut bot = event();
    bot["event_id"] = json!("EvBot");
    bot["event"]["bot_id"] = json!("B1");
    app.clone().oneshot(signed(bot, true)).await.unwrap();
    assert_eq!(t.count("slack_jobs"), 1);
    let challenge = json!({"type":"url_verification","challenge":"challenge"});
    assert_eq!(
        app.oneshot(signed(challenge, true)).await.unwrap().status(),
        StatusCode::OK
    );
    let mut off = t.state.clone();
    off.providers.slack_agent = None;
    assert_ne!(
        router(off)
            .oneshot(signed(event(), true))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(SlackAgent::from_env(&|_| None).unwrap().is_none());
}

#[tokio::test]
async fn slack_context_becomes_one_request_and_only_owner_approval_releases_result() {
    let t = Test::new().await;
    t.mock
        .thread
        .lock()
        .unwrap()
        .push(json!({"user":"U1","ts":"999.000001","text":"Which PRs fixed Lens loading?"}));
    t.push(vec![tool("read_thread",json!({"ts":"999.000001"})),tool("list_people",json!({})),tool("create_request",json!({"agent_id":t.agent,"request":"Find the Lens optimization PRs and whether they shipped."}))]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvFlow"))
        .unwrap();
    let mut job = t.tick().await;
    for _ in 0..6 {
        job = t.tick().await;
    }
    let task_id = job.task.clone().expect("request created");
    assert_eq!(t.count("tasks"), 1);
    assert_eq!(t.count("approvals"), 0);
    let task = t
        .state
        .db()
        .unwrap()
        .get_task(&t.requester, &task_id)
        .unwrap();
    assert_eq!(task.requester_id, t.requester.user_id);
    assert!(task.request.contains("everyone in that Slack channel"));
    assert!(task.request.contains("Which PRs fixed Lens loading?"));
    assert_eq!(task.status, Status::Queued);
    t.tick().await; // Sent, awaiting approval.
    t.clock.fetch_add(6, Ordering::SeqCst);
    let work = t
        .state
        .db()
        .unwrap()
        .claim(&t.worker, &t.agent)
        .unwrap()
        .unwrap();
    assert_eq!(work.phase, "prepare");
    let mut action = action();
    if let Action::Command { share_result, .. } = &mut action {
        *share_result = true;
    }
    let prepared = t
        .state
        .db()
        .unwrap()
        .prepare(
            &t.worker,
            &task_id,
            Some(&work.lease),
            DraftInput {
                text: String::new(),
                attachment_ids: vec![],
                action: Some(action),
            },
        )
        .unwrap();
    assert!(
        t.state
            .db()
            .unwrap()
            .claim(&t.worker, &t.agent)
            .unwrap()
            .is_none()
    );
    let slack_p = t
        .state
        .db()
        .unwrap()
        .slack_principal("T1", "U1")
        .unwrap()
        .unwrap();
    let draft = prepared.draft.unwrap();
    assert!(
        t.state
            .db()
            .unwrap()
            .decide(
                &slack_p,
                &task_id,
                draft.version,
                &draft.digest,
                Decision::Approve
            )
            .is_err()
    );
    t.state
        .db()
        .unwrap()
        .decide(
            &t.owner,
            &task_id,
            draft.version,
            &draft.digest,
            Decision::Approve,
        )
        .unwrap();
    t.tick().await; // Approved, waiting for the worker.
    t.clock.fetch_add(6, Ordering::SeqCst);
    let work = t
        .state
        .db()
        .unwrap()
        .claim(&t.worker, &t.agent)
        .unwrap()
        .unwrap();
    assert_eq!(work.phase, "execute");
    t.state
        .db()
        .unwrap()
        .stage_result(
            &t.worker,
            &task_id,
            &work.lease,
            DraftInput {
                text: "The fixes are in PR #55 and #66.".into(),
                ..Default::default()
            },
        )
        .unwrap();
    t.tick().await;
    t.tick().await; // Final answer is a new reply after the status update.
    let calls = t.mock.calls.lock().unwrap();
    let posted = calls
        .iter()
        .rev()
        .find(|(path, _)| path == "/chat.postMessage")
        .unwrap();
    assert_eq!(posted.1["channel"], "C1");
    assert_eq!(posted.1["thread_ts"], "999.000001");
    assert!(posted.1["text"].as_str().unwrap().contains("PR #55"));
    let initial = calls
        .iter()
        .find(|(path, _)| path == "/chat.postMessage")
        .unwrap();
    assert_ne!(initial.1["client_msg_id"], posted.1["client_msg_id"]);
    assert!(calls.iter().any(|(path, body)| path == "/chat.update"
        && body["text"].as_str().unwrap().contains("Moe approved")));
    assert_eq!(
        calls
            .iter()
            .filter(|(path, _)| path == "/chat.postMessage")
            .count(),
        2
    );
    for (path, body) in calls
        .iter()
        .filter(|(path, _)| path == "/v1/chat/completions")
    {
        assert_eq!(path, "/v1/chat/completions");
        assert!(body.get("max_tokens").is_none() && body.get("max_completion_tokens").is_none());
        assert!(!body.to_string().contains("private-slack-token"));
        assert!(!body.to_string().contains("The fixes are in PR #55")); // result bypasses model
    }
}

#[tokio::test]
async fn injected_tools_and_foreign_recipient_cannot_act_or_change_sender() {
    let t = Test::new().await;
    link(&t.state.db().unwrap(), &t.owner, "U2", "OTHERTEAM");
    t.push(vec![tool("approve",json!({"decision":"approve"})),tool("create_request",json!({"agent_id":t.agent,"request":"Do it now","user_id":t.owner.user_id,"channel":"SECRET"})),json!({"role":"assistant","content":"Please mention someone linked to this workspace, then @Litecom again."})]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvInjection"))
        .unwrap();
    for _ in 0..7 {
        t.tick().await;
    }
    assert_eq!(t.count("tasks"), 0);
    assert_eq!(t.count("approvals"), 0);
    assert!(
        t.mock
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, p)| p.get("channel").is_none_or(|c| c == "C1"))
    );
}

#[tokio::test]
async fn unlinked_sender_gets_connect_link_without_model_access() {
    let t = Test::new().await;
    let mut m = mention("EvUnknown");
    m.user = "UNKNOWN".into();
    t.state.db().unwrap().enqueue_slack(m).unwrap();
    t.tick().await;
    t.tick().await;
    assert_eq!(t.count("tasks"), 0);
    let calls = t.mock.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "/chat.postMessage");
    assert!(calls[0].1["text"].as_str().unwrap().contains("/settings"));
}

#[tokio::test]
async fn crashed_tool_step_reuses_the_same_request_and_claims_expire() {
    let t = Test::new().await;
    t.push(vec![tool(
        "create_request",
        json!({"agent_id":t.agent,"request":"Find the PRs."}),
    )]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvCrash"))
        .unwrap();
    t.tick().await;
    t.tick().await;
    let mut abandoned = t.state.db().unwrap().claim_slack().unwrap().unwrap();
    step(&t.state, &mut abandoned).await.unwrap(); // task committed, job progress lost
    let first = abandoned.task.clone().unwrap();
    assert!(t.state.db().unwrap().claim_slack().unwrap().is_none());
    t.clock.fetch_add(121, Ordering::SeqCst);
    let resumed = t.tick().await;
    assert_eq!(resumed.task.as_deref(), Some(first.as_str()));
    assert_eq!(t.count("tasks"), 1);
    assert!(
        t.state
            .db()
            .unwrap()
            .save_slack(&abandoned, false, 0)
            .is_err()
    );
}

#[tokio::test]
async fn channel_reads_ignore_model_supplied_destinations_and_preserve_context() {
    let t = Test::new().await;
    t.mock
        .thread
        .lock()
        .unwrap()
        .push(json!({"user":"U1","text":"Original question above","ts":"998.000001"}));
    t.push(vec![tool("read_channel",json!({"channel":"PRIVATE","latest":"9999.000001"})),json!({"role":"assistant","content":"Which Moe? Reply with @Litecom and their Slack mention."})]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvRead"))
        .unwrap();
    let mut job = t.tick().await;
    for _ in 0..4 {
        job = t.tick().await;
    }
    assert!(
        serde_json::to_string(&job.context)
            .unwrap()
            .contains("Original question above")
    );
    let calls = t.mock.calls.lock().unwrap();
    let read = calls
        .iter()
        .find(|(p, _)| p == "/conversations.history")
        .unwrap();
    assert_eq!(read.1["channel"], "C1");
    assert_eq!(read.1["latest"], "1000.000001");
    assert_eq!(t.count("tasks"), 0);
}

#[test]
fn encrypted_intake_survives_restart_and_is_quarantined_on_restore() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("relay.db");
    let clock = Arc::new(AtomicI64::new(1000));
    let open = || {
        let clock = clock.clone();
        Store::with_clock(
            &path,
            [8; 32],
            Arc::new(move || clock.load(Ordering::SeqCst)),
        )
        .unwrap()
    };
    let store = open();
    let mut m = mention("EvRestart");
    m.text = "private source text".into();
    store.enqueue_slack(m.clone()).unwrap();
    let job = store.claim_slack().unwrap().unwrap();
    let bytes: Vec<u8> = store
        .db
        .query_row("SELECT data FROM slack_jobs", [], |r| r.get(0))
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private source text"));
    drop(store);
    let store = open();
    assert!(store.claim_slack().unwrap().is_none());
    clock.fetch_add(121, Ordering::SeqCst);
    let restored = store.claim_slack().unwrap().unwrap();
    assert_eq!(restored.mention.text, "private source text");
    assert_ne!(restored.claim, job.claim);
    store.enqueue_slack(m).unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM slack_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    store.quarantine_restored_state().unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM slack_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn failed_slack_delivery_retries_without_rerunning_model_or_request() {
    let t = Test::new().await;
    t.push(vec![json!({"role":"assistant","content":"Which person do you mean? Reply with @Litecom and their Slack mention."})]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvRetry"))
        .unwrap();
    t.tick().await;
    t.tick().await;
    let mut failed = t.state.db().unwrap().claim_slack().unwrap().unwrap();
    t.mock.fail_post.store(true, Ordering::SeqCst);
    assert!(step(&t.state, &mut failed).await.is_err());
    t.state.db().unwrap().retry_slack(&failed).unwrap();
    assert!(t.state.db().unwrap().claim_slack().unwrap().is_none());
    t.clock.fetch_add(31, Ordering::SeqCst);
    t.tick().await;
    let calls = t.mock.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|(p, _)| p == "/v1/chat/completions")
            .count(),
        1
    );
    let posts = calls
        .iter()
        .filter(|(p, _)| p == "/chat.postMessage")
        .collect::<Vec<_>>();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].1["client_msg_id"], posts[1].1["client_msg_id"]);
    assert_eq!(t.count("tasks"), 0);
}

#[tokio::test]
async fn long_slack_replies_are_split_without_losing_unicode_or_reprocessing() {
    let t = Test::new().await;
    let text = "🦀".repeat(40_001);
    t.push(vec![json!({"role":"assistant","content":text})]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvLong"))
        .unwrap();
    t.tick().await;
    t.tick().await;
    let job = t.tick().await;
    assert_eq!(job.remote.len(), 3);
    let calls = t.mock.calls.lock().unwrap();
    let chunks = calls
        .iter()
        .filter(|(p, _)| p == "/chat.postMessage")
        .map(|(_, body)| body["text"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        chunks
            .iter()
            .all(|text| text.encode_utf16().count() <= 40_000)
    );
    assert_eq!(chunks.join(""), text);
    assert!(t.state.db().unwrap().claim_slack().unwrap().is_none());
}

#[tokio::test]
async fn blocked_people_cannot_receive_requests_and_self_requests_still_need_approval() {
    let t = Test::new().await;
    t.state
        .db()
        .unwrap()
        .block_contact(&t.owner, &t.requester.user_id)
        .unwrap();
    t.push(vec![
        tool(
            "create_request",
            json!({"agent_id":t.agent,"request":"Find the PRs."}),
        ),
        json!({"role":"assistant","content":"That person is not available."}),
    ]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvBlocked"))
        .unwrap();
    for _ in 0..5 {
        t.tick().await;
    }
    assert_eq!(t.count("tasks"), 0);
    let self_agent = t
        .state
        .db()
        .unwrap()
        .agents(&t.requester, &t.requester.user_id)
        .unwrap()[0]
        .id
        .clone();
    t.push(vec![tool(
        "create_request",
        json!({"agent_id":self_agent,"request":"Find my notes."}),
    )]);
    t.state
        .db()
        .unwrap()
        .enqueue_slack(mention("EvSelf"))
        .unwrap();
    t.tick().await;
    t.tick().await;
    let job = t.tick().await;
    let task = t
        .state
        .db()
        .unwrap()
        .get_task(&t.requester, job.task.as_deref().unwrap())
        .unwrap();
    assert_eq!(task.requester_id, task.owner_id);
    assert_eq!(task.status, Status::Queued);
    assert_eq!(t.count("approvals"), 0);
}

#[tokio::test]
#[ignore = "Uses the explicitly configured SLACK_AGENT_* model provider and its billing; Slack and execution remain mocked"]
async fn configured_model_resolves_a_contextual_slack_request() {
    let mut t = Test::new().await;
    t.state.providers.slack_agent =
        SlackAgent::from_env(&|key| std::env::var(key).ok().filter(|s| !s.is_empty())).unwrap();
    assert!(
        t.state.providers.slack_agent.is_some(),
        "Set SLACK_AGENT_BASE_URL, SLACK_AGENT_MODEL and the provider's API key"
    );
    t.mock.thread.lock().unwrap().extend([
        json!({"user":"U1","ts":"998.000001","text":"We're checking whether the Lens loading optimizations shipped. We need the pull request links and whether they were merged."}),
        json!({"user":"U2","ts":"999.000001","text":"My agent has the project context and can check those PRs after I approve."}),
    ]);
    let mut m = mention("EvModelSmoke");
    m.text = "@Litecom can you get this from Moe ^".into();
    t.state.db().unwrap().enqueue_slack(m).unwrap();
    let task = tokio::time::timeout(std::time::Duration::from_secs(180), async {
        loop {
            let job = t.tick().await;
            if let Some(task) = job.task {
                break task;
            }
            assert!(
                job.reply.is_none(),
                "The model asked for clarification or could not complete this unambiguous fixture"
            );
        }
    })
    .await
    .expect("The model did not finish the handoff");
    let request = t.state.db().unwrap().get_task(&t.requester, &task).unwrap();
    assert_eq!(request.owner_id, t.owner.user_id);
    assert_eq!(request.agent_id, t.agent);
    assert_eq!(request.status, Status::Queued);
    assert!(request.request.contains("Lens"));
    assert_eq!(t.count("approvals"), 0);
    assert!(
        t.mock
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/conversations.replies" || p == "/conversations.history")
    );
}
