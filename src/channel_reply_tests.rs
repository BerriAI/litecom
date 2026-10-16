use super::*;
use crate::{
    api::{AppState, router},
    channel_replies::{ChannelActor, ReplyOutcome},
    notifications::Delivery,
    providers::Providers,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hmac::{Hmac, Mac};
use tower::ServiceExt;

fn destination(f: &Fixture, kind: &str) -> Delivery {
    let value = match kind {
        "phone" => "+14155550102".into(),
        "email" => "moe@example.com".into(),
        _ => serde_json::to_string(&crate::slack::SlackDestination {
            token: "secret".into(),
            refresh_token: None,
            expires_at: None,
            user: "U1".into(),
            team: "T1".into(),
            channel: "D1".into(),
        })
        .unwrap(),
    };
    f.store.verified_channel(&f.b, kind, &value).unwrap();
    let channel = f
        .store
        .channels(&f.b)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == kind)
        .unwrap();
    Delivery {
        event_id: 0,
        channel_id: channel.id,
        destination: value,
        task_id: String::new(),
        kind: "awaiting_approval".into(),
        attempts: 1,
        channel_kind: kind.into(),
        previous: None,
    }
}
fn slack_actor() -> ChannelActor {
    ChannelActor::Slack {
        team: "T1".into(),
        user: "U1".into(),
        channel: "D1".into(),
    }
}

#[test]
fn all_approval_channels_enforce_current_workspace_membership() {
    for kind in ["slack", "email", "phone"] {
        let mut f = Fixture::new();
        let mut delivery = destination(&f, kind);
        let task = f.draft("workspace-policy", Some(action()));
        delivery.task_id = task.id.clone();
        let approval = f.store.channel_action(&delivery).unwrap().unwrap();
        let (actor, key) = match kind {
            "slack" => (slack_actor(), approval.token),
            "email" => (
                ChannelActor::Email("moe@example.com".into()),
                approval.token,
            ),
            _ => (ChannelActor::Phone("+14155550102".into()), approval.code),
        };
        f.store.workspace_policy = Some(crate::google::Google::new(
            "test-client".into(),
            "secret".into(),
            "example.com",
        ));
        assert!(
            f.store
                .channel_decide(&actor, &key, Decision::Approve)
                .is_err()
        );
        assert_eq!(count(&f.store, &task.id), 0);
        super::security_tests::allow_member(&f.store, &f.b.user_id);
        super::security_tests::allow_member(&f.store, &f.a.user_id);
        f.store
            .channel_decide(&actor, &key, Decision::Approve)
            .unwrap();
        assert_eq!(count(&f.store, &task.id), 1);
    }
}
fn count(store: &Store, task: &str) -> i64 {
    store
        .db
        .query_row(
            "SELECT COUNT(*) FROM approvals WHERE task_id=?",
            [task],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn channels_share_one_decision_and_replays_do_not_repeat_execution() {
    let f = Fixture::new();
    let mut phone = destination(&f, "phone");
    let mut email = destination(&f, "email");
    let mut slack = destination(&f, "slack");
    let task = f.draft("direct", Some(action()));
    phone.task_id = task.id.clone();
    email.task_id = task.id.clone();
    slack.task_id = task.id.clone();
    let p = f.store.channel_action(&phone).unwrap().unwrap();
    let e = f.store.channel_action(&email).unwrap().unwrap();
    let s = f.store.channel_action(&slack).unwrap().unwrap();
    assert_eq!(
        p.token,
        f.store.channel_action(&phone).unwrap().unwrap().token
    );
    assert!(p.body.contains("From: Theo") && p.body.contains(&task.request));
    assert!(
        f.store
            .channel_decide(
                &ChannelActor::Phone("+14155550101".into()),
                &p.code,
                Decision::Approve
            )
            .is_err()
    );
    assert!(
        f.store
            .channel_decide(
                &ChannelActor::Email("evil@example.com".into()),
                &e.token,
                Decision::Approve
            )
            .is_err()
    );
    assert!(
        f.store
            .channel_decide(
                &ChannelActor::Slack {
                    team: "T1".into(),
                    user: "U2".into(),
                    channel: "D1".into()
                },
                &s.token,
                Decision::Approve
            )
            .is_err()
    );
    assert!(matches!(
        f.store
            .channel_decide(&slack_actor(), &s.token, Decision::Approve)
            .unwrap(),
        ReplyOutcome::Decided(Decision::Approve)
    ));
    assert!(matches!(
        f.store
            .channel_decide(
                &ChannelActor::Phone("+14155550102".into()),
                &p.code,
                Decision::Decline
            )
            .unwrap(),
        ReplyOutcome::AlreadyDecided(Decision::Approve)
    ));
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    f.store
        .stage_result(
            &f.bw,
            &task.id,
            &work.lease,
            DraftInput {
                text: "Done".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(
        f.store
            .channel_decide(
                &ChannelActor::Email("moe@example.com".into()),
                &e.token,
                Decision::Approve
            )
            .unwrap(),
        ReplyOutcome::AlreadyDecided(Decision::Approve)
    ));
    assert_eq!(count(&f.store, &task.id), 1);
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    assert!(f.store.channel_action(&slack).unwrap().is_none());
    let encrypted: Vec<u8> = f
        .store
        .db
        .query_row(
            "SELECT data FROM channel_actions WHERE token_hash=?",
            [hash(&s.token)],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains(&s.token));
    // Both owner and requester receive an update from the same committed decision.
    let updates:i64=f.store.db.query_row("SELECT COUNT(*) FROM deliveries d JOIN events e ON e.sequence=d.event_id WHERE e.task_id=? AND e.kind='approved'",[task.id],|r|r.get(0)).unwrap();
    assert_eq!(updates, 3);
}

#[test]
fn changed_cancelled_disconnected_and_other_peoples_requests_cannot_be_approved() {
    for mode in ["changed", "cancelled", "disabled", "reconnected"] {
        let f = Fixture::new();
        let mut d = destination(&f, "email");
        let task = f.draft(mode, Some(action()));
        d.task_id = task.id.clone();
        let a = f.store.channel_action(&d).unwrap().unwrap();
        match mode {
            "changed" => {
                f.store
                    .prepare(
                        &f.b,
                        &task.id,
                        None,
                        DraftInput {
                            text: "Changed".into(),
                            action: Some(action()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
            }
            "cancelled" => {
                f.store.cancel(&f.a, &task.id).unwrap();
            }
            "disabled" => f.store.disable_channel(&f.b, &d.channel_id).unwrap(),
            _ => f
                .store
                .verified_channel(&f.b, "email", "new@example.com")
                .unwrap(),
        }
        assert!(
            f.store
                .channel_decide(
                    &ChannelActor::Email("moe@example.com".into()),
                    &a.token,
                    Decision::Approve
                )
                .is_err(),
            "{mode}"
        );
        assert_eq!(count(&f.store, &task.id), 0);
    }
    let f = Fixture::new();
    let mut d = destination(&f, "phone");
    let task = f.draft("owner-only", Some(action()));
    d.task_id = task.id.clone();
    f.store
        .verified_channel(&f.a, "phone", "+14155550101")
        .unwrap();
    d.channel_id = f.store.channels(&f.a).unwrap()[0].id.clone();
    d.destination = "+14155550101".into();
    assert!(f.store.channel_action(&d).unwrap().is_none());
}

#[test]
fn direct_tool_approval_is_once_only_and_cannot_approve_the_next_tool() {
    let f = Fixture::new();
    let mut d = destination(&f, "phone");
    let task = f.draft("tool", Some(action()));
    d.task_id = task.id.clone();
    f.approve(&task);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let p = f
        .store
        .request_permission(
            &f.bw,
            &task.id,
            &work.lease,
            ToolPermissionInput {
                id: id(),
                title: "Run test".into(),
                details: serde_json::json!({"command":"cargo test"}),
            },
        )
        .unwrap();
    let token = f.store.channel_action(&d).unwrap().unwrap();
    f.store
        .channel_decide(
            &ChannelActor::Phone("+14155550102".into()),
            &token.code,
            Decision::Approve,
        )
        .unwrap();
    assert_eq!(
        f.store
            .permission_status(&f.bw, &task.id, &work.lease, &p.id)
            .unwrap()
            .decision,
        Some(Decision::Approve)
    );
    let p2 = f
        .store
        .request_permission(
            &f.bw,
            &task.id,
            &work.lease,
            ToolPermissionInput {
                id: id(),
                title: "Delete file".into(),
                details: serde_json::json!({"file":"important.txt"}),
            },
        )
        .unwrap();
    assert!(matches!(
        f.store
            .channel_decide(
                &ChannelActor::Phone("+14155550102".into()),
                &token.code,
                Decision::Approve
            )
            .unwrap(),
        ReplyOutcome::AlreadyDecided(_)
    ));
    assert!(
        f.store
            .permission_status(&f.bw, &task.id, &work.lease, &p2.id)
            .unwrap()
            .decision
            .is_none()
    );
    let second = f.store.channel_action(&d).unwrap().unwrap();
    assert_ne!(token.code, second.code);
    f.store.cancel(&f.a, &task.id).unwrap();
    assert!(
        f.store
            .channel_decide(
                &ChannelActor::Phone("+14155550102".into()),
                &second.code,
                Decision::Approve
            )
            .is_err()
    );
}

type PhoneMessages = std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>;
async fn phone_server() -> (String, PhoneMessages, tokio::task::JoinHandle<()>) {
    let messages: PhoneMessages = Default::default();
    let saved = messages.clone();
    let app = axum::Router::new().route(
        "/v1/messages",
        axum::routing::post(
            move |headers: axum::http::HeaderMap,
                  axum::Json(body): axum::Json<serde_json::Value>| {
                let saved = saved.clone();
                async move {
                    assert_eq!(headers["authorization"], "Bearer phone-key");
                    assert_eq!(body["agent_id"], "agt_test");
                    assert_eq!(body["number_id"], "num_test");
                    saved.lock().unwrap().push(body);
                    axum::Json(
                        serde_json::json!({"id":"msg_test","status":"sent","channel":"imessage"}),
                    )
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, messages, task)
}
fn phone_event(command: &str, sender: &str) -> serde_json::Value {
    serde_json::json!({"event":"agent.message","agentId":"agt_test","channel":"sms",
        "timestamp":"2026-10-15T18:00:00Z", "data":{"numberId":"num_test", "from":sender,
        "to":"+14155550999","message":command,"direction":"inbound","receivedAt":"2026-10-15T18:00:00Z"}})
}
fn signed_phone_request(event: &serde_json::Value, time: i64) -> Request<Body> {
    let body = event.to_string();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"phone-secret").unwrap();
    mac.update(format!("{time}.{body}").as_bytes());
    Request::post("/webhooks/agentphone")
        .header("content-type", "application/json")
        .header("x-webhook-timestamp", time.to_string())
        .header("x-webhook-id", "delivery-fixture")
        .header(
            "x-webhook-signature",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        )
        .body(Body::from(body))
        .unwrap()
}
#[tokio::test]
async fn phone_callback_needs_signature_sender_and_exact_code() {
    let (url, messages, server) = phone_server().await;
    let f = Fixture::new();
    let mut d = destination(&f, "phone");
    f.store.enable_phone(&f.a).unwrap();
    let task = f.draft("phone-http", Some(action()));
    d.task_id = task.id.clone();
    let a = f.store.channel_action(&d).unwrap().unwrap();
    let state = AppState::new(
        f.store,
        "https://relay.example".into(),
        Providers::test_replies(&url, "slack-secret"),
    );
    let app = router(state.clone());
    let event = phone_event(&format!("APPROVE {}", a.code), "+14155550102");
    let mut request = signed_phone_request(&event, crate::store::now());
    request
        .headers_mut()
        .insert("x-webhook-signature", "sha256=abcd".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    for event in [
        phone_event(&format!("APPROVE {}", a.code), "+14155550101"),
        phone_event("Approve", "+14155550102"),
    ] {
        let response = app
            .clone()
            .oneshot(signed_phone_request(&event, crate::store::now()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(count(&state.db().unwrap(), &task.id), 0);
    }
    // Invalid replies receive helpful text through the actual provider transport, not the HTTP response body.
    assert_eq!(messages.lock().unwrap().len(), 2);
    assert!(
        messages.lock().unwrap()[1]["body"]
            .as_str()
            .unwrap()
            .contains("APPROVE")
    );
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(signed_phone_request(&event, crate::store::now()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(count(&state.db().unwrap(), &task.id), 1);
    }
    assert_eq!(
        messages.lock().unwrap().len(),
        2,
        "Successful approval uses the committed notification outbox"
    );
    let work = state.db().unwrap().claim(&f.bw, &f.agent).unwrap().unwrap();
    state
        .db()
        .unwrap()
        .stage_result(
            &f.bw,
            &task.id,
            &work.lease,
            DraftInput {
                text: "Done".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        state.db().unwrap().get_task(&f.a, &task.id).unwrap().status,
        Status::Completed
    );
    assert_eq!(count(&state.db().unwrap(), &task.id), 1);
    // A fresh reply after completion reports the earlier decision without repeating work.
    let mut repeat = event.clone();
    repeat["timestamp"] = "2026-10-15T18:01:00Z".into();
    app.clone()
        .oneshot(signed_phone_request(&repeat, crate::store::now()))
        .await
        .unwrap();
    assert!(
        messages.lock().unwrap().last().unwrap()["body"]
            .as_str()
            .unwrap()
            .contains("already approved")
    );
    let stop = phone_event("STOP", "+14155550102");
    assert_eq!(
        app.clone()
            .oneshot(signed_phone_request(&stop, crate::store::now()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(!state.db().unwrap().channels(&f.b).unwrap()[0].verified);
    state.db().unwrap().enable_phone(&f.b).unwrap();
    // Replayed STOP must not disable a channel the human has deliberately reconnected.
    app.oneshot(signed_phone_request(&stop, crate::store::now()))
        .await
        .unwrap();
    assert!(state.db().unwrap().channels(&f.b).unwrap()[0].verified);
    server.abort();
}
#[tokio::test]
async fn phone_rejects_cross_agent_expired_group_voice_reactions_and_tampered_replies() {
    let (url, messages, server) = phone_server().await;
    let f = Fixture::new();
    let mut d = destination(&f, "phone");
    let task = f.draft("phone-boundaries", Some(action()));
    d.task_id = task.id.clone();
    let a = f.store.channel_action(&d).unwrap().unwrap();
    let state = AppState::new(
        f.store,
        "https://relay.example".into(),
        Providers::test_replies(&url, "slack-secret"),
    );
    let app = router(state.clone());
    let original = phone_event(&format!("APPROVE {}", a.code), "+14155550102");
    // Allow for the wall clock advancing between signing and verification.
    // agentphone::tests pins the clock to cover the exact 300-second boundary.
    for age in [-600, 600] {
        let response = app
            .clone()
            .oneshot(signed_phone_request(&original, crate::store::now() + age))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    for (pointer, value, status) in [
        (
            "/agentId",
            serde_json::json!("agt_other"),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/data/numberId",
            serde_json::json!("num_other"),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/data/from",
            serde_json::json!("moe@example.com"),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/data/direction",
            serde_json::json!("outbound"),
            StatusCode::OK,
        ),
        ("/channel", serde_json::json!("voice"), StatusCode::OK),
        ("/channel", serde_json::json!("whatsapp"), StatusCode::OK),
        (
            "/event",
            serde_json::json!("agent.reaction"),
            StatusCode::OK,
        ),
    ] {
        let mut event = original.clone();
        *event.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            app.clone()
                .oneshot(signed_phone_request(&event, crate::store::now()))
                .await
                .unwrap()
                .status(),
            status,
            "{pointer}"
        );
    }
    for group in [
        serde_json::json!({"isGroup":true}),
        serde_json::json!({"isGroup":false}),
        serde_json::json!({}),
    ] {
        let mut event = original.clone();
        event["data"]["group"] = group;
        assert_eq!(
            app.clone()
                .oneshot(signed_phone_request(&event, crate::store::now()))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    let mut event = original.clone();
    event["data"]["senderIdentifier"] = "+14155550102".into();
    app.clone()
        .oneshot(signed_phone_request(&event, crate::store::now()))
        .await
        .unwrap();
    let mut tampered = signed_phone_request(&original, crate::store::now());
    *tampered.body_mut() = Body::from(event.to_string());
    assert_eq!(
        app.clone().oneshot(tampered).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(count(&state.db().unwrap(), &task.id), 0);
    assert!(messages.lock().unwrap().is_empty());
    // iMessage uses the same verified phone account and exact request code.
    let mut imessage = original;
    imessage["channel"] = "imessage".into();
    app.oneshot(signed_phone_request(&imessage, crate::store::now()))
        .await
        .unwrap();
    assert_eq!(count(&state.db().unwrap(), &task.id), 1);
    server.abort();
}

#[tokio::test]
async fn phone_receipts_survive_restart_and_disabled_channels_receive_no_replies() {
    let (url, messages, server) = phone_server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = Store::open(&path, [7; 32]).unwrap();
    let (human, _) = user(&store, "Moe", "+14155550102");
    store.enable_phone(&human).unwrap();
    let state = AppState::new(
        store,
        "https://relay.example".into(),
        Providers::test_replies(&url, "slack-secret"),
    );
    let stop = phone_event("STOP", "+14155550102");
    let response = router(state.clone())
        .oneshot(signed_phone_request(&stop, crate::store::now()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!state.db().unwrap().channels(&human).unwrap()[0].verified);
    let help = phone_event("Hello", "+14155550102");
    router(state.clone())
        .oneshot(signed_phone_request(&help, crate::store::now()))
        .await
        .unwrap();
    assert!(messages.lock().unwrap().is_empty());
    state.db().unwrap().enable_phone(&human).unwrap();
    drop(state);
    let state = AppState::new(
        Store::open(&path, [7; 32]).unwrap(),
        "https://relay.example".into(),
        Providers::test_replies(&url, "slack-secret"),
    );
    let mut replay = signed_phone_request(&stop, crate::store::now());
    // This unsigned header cannot make a previously handled body new again.
    replay
        .headers_mut()
        .insert("x-webhook-id", "changed-delivery-id".parse().unwrap());
    router(state.clone()).oneshot(replay).await.unwrap();
    assert!(state.db().unwrap().channels(&human).unwrap()[0].verified);
    router(state.clone())
        .oneshot(signed_phone_request(&help, crate::store::now()))
        .await
        .unwrap();
    assert_eq!(messages.lock().unwrap().len(), 1);
    router(state)
        .oneshot(signed_phone_request(&help, crate::store::now()))
        .await
        .unwrap();
    assert_eq!(messages.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn slack_callback_requires_signed_fresh_exact_user_and_channel() {
    let f = Fixture::new();
    let mut d = destination(&f, "slack");
    let task = f.draft("slack-http", Some(action()));
    d.task_id = task.id.clone();
    let a = f.store.channel_action(&d).unwrap().unwrap();
    let state = AppState::new(
        f.store,
        "https://relay.example".into(),
        Providers::test_replies("http://127.0.0.1:1", "slack-secret"),
    );
    let app = router(state.clone());
    for (age, user, channel, ok) in [
        (301, "U1", "D1", false),
        (0, "U2", "D1", false),
        (0, "U1", "D2", false),
        (0, "U1", "D1", true),
    ] {
        let payload=serde_json::json!({"type":"block_actions","user":{"id":user},"team":{"id":"T1"},"channel":{"id":channel},"actions":[{"action_id":"litecom_approve","value":a.token}],"response_url":"http://127.0.0.1/never-follow-this"}).to_string();
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("payload", &payload)
            .finish();
        let time = crate::store::now() - age;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"slack-secret").unwrap();
        mac.update(format!("v0:{time}:{body}").as_bytes());
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
        let response = app
            .clone()
            .oneshot(
                Request::post("/webhooks/slack")
                    .header("x-slack-request-timestamp", time.to_string())
                    .header("x-slack-signature", signature)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        if ok {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(count(&state.db().unwrap(), &task.id), 1);
        } else {
            assert_eq!(count(&state.db().unwrap(), &task.id), 0);
        }
    }
}
#[test]
fn email_requires_reply_capability_and_connected_sender() {
    let f = Fixture::new();
    let mut d = destination(&f, "email");
    let task = f.draft("email", Some(action()));
    d.task_id = task.id.clone();
    let a = f.store.channel_action(&d).unwrap().unwrap();
    let state = AppState::new(
        f.store,
        "https://relay.example".into(),
        Providers::development().unwrap(),
    );
    let message = format!(
        "From: moe@example.com\r\nIn-Reply-To: <litecom-approve-{}@relay.example>\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<div>Approve</div><div><br></div><blockquote>Decline</blockquote>",
        a.token
    );
    assert!(
        !crate::email_replies::process(
            &state,
            message
                .replace("moe@example.com", "attacker@example.com")
                .as_bytes()
        )
        .unwrap()
    );
    assert!(
        !crate::email_replies::process(&state, message.replace(&a.token, &secret()).as_bytes())
            .unwrap()
    );
    assert_eq!(count(&state.db().unwrap(), &task.id), 0);
    assert!(crate::email_replies::process(&state, message.as_bytes()).unwrap());
    assert!(crate::email_replies::process(&state, message.as_bytes()).unwrap());
    assert_eq!(count(&state.db().unwrap(), &task.id), 1);
}

#[tokio::test]
async fn channel_read_only_decisions_downgrade_actions_and_replays_cannot_escalate() {
    for kind in ["slack", "email", "phone"] {
        let f = Fixture::new();
        let mut delivery = destination(&f, kind);
        let settings = crate::runner::RunnerConfig {
            agent: crate::runner::NativeAgent::Codex,
            executable: std::env::current_exe().unwrap(),
            workspace: std::env::temp_dir(),
            arguments: vec![],
            mcp: None,
        };
        let task = f.draft("downgrade", Some(settings.action("Create a file")));
        delivery.task_id = task.id.clone();
        let card = f.store.channel_action(&delivery).unwrap().unwrap();
        assert!(card.read_only && card.actions);
        let (url, _, server) = phone_server().await;
        let state = AppState::new(
            f.store,
            "https://relay.example".into(),
            Providers::test_replies(&url, "slack-secret"),
        );
        let app = router(state.clone());
        match kind {
            "slack" => {
                let payload = serde_json::json!({"type":"block_actions","user":{"id":"U1"},"team":{"id":"T1"},"channel":{"id":"D1"},"actions":[{"action_id":"litecom_read_only","value":card.token}]}).to_string();
                let body = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("payload", &payload)
                    .finish();
                let time = crate::store::now();
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"slack-secret").unwrap();
                mac.update(format!("v0:{time}:{body}").as_bytes());
                let response = app
                    .oneshot(
                        Request::post("/webhooks/slack")
                            .header("x-slack-request-timestamp", time.to_string())
                            .header(
                                "x-slack-signature",
                                format!("v0={}", hex::encode(mac.finalize().into_bytes())),
                            )
                            .body(Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
            }
            "email" => {
                let mail = format!(
                    "From: moe@example.com\r\nIn-Reply-To: <litecom-approve-{}@relay.example>\r\nContent-Type: text/plain\r\n\r\nRead-only",
                    card.token
                );
                assert!(crate::email_replies::process(&state, mail.as_bytes()).unwrap());
            }
            _ => {
                let event = phone_event(&format!("READONLY {}", card.code), "+14155550102");
                assert_eq!(
                    app.oneshot(signed_phone_request(&event, crate::store::now()))
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::OK
                );
            }
        }
        let (actor, key) = if kind == "phone" {
            (ChannelActor::Phone("+14155550102".into()), &card.code)
        } else if kind == "email" {
            (ChannelActor::Email("moe@example.com".into()), &card.token)
        } else {
            (slack_actor(), &card.token)
        };
        state
            .db()
            .unwrap()
            .channel_decide(&actor, key, Decision::Approve)
            .unwrap();
        let approved = state.db().unwrap().get_task(&f.b, &task.id).unwrap();
        assert_eq!(approved.approved_access, Some(RunAccess::ReadOnly));
        assert!(approved.draft.unwrap().action.unwrap().read_only());
        assert_eq!(count(&state.db().unwrap(), &task.id), 1);
        server.abort();
    }
}

#[test]
fn slack_cards_keep_ask_from_and_where_after_the_decision_without_dumping_context() {
    for decision in [Decision::Approve, Decision::Decline] {
        let f = Fixture::new();
        let mut delivery = destination(&f, "slack");
        let request = f
            .store
            .create_task_with_source(
                &f.a,
                NewTask {
                    ask_only: false,
                    owner_id: f.b.user_id.clone(),
                    agent_id: Some(f.agent.clone()),
                    request: "Find the Lens PRs. Quoted Slack context: private context dump".into(),
                    idempotency_key: "concise-card".into(),
                },
                Some(crate::model::RequestSource {
                    ask: "Find the Lens performance PRs.".into(),
                    url: "https://example.slack.com/archives/C1/p1000000001".into(),
                    share_files: true,
                }),
            )
            .unwrap();
        delivery.task_id = request.id.clone();
        let settings = crate::runner::RunnerConfig {
            agent: crate::runner::NativeAgent::Codex,
            executable: std::env::current_exe().unwrap(),
            workspace: std::env::temp_dir(),
            arguments: vec![],
            mcp: None,
        };
        f.store
            .prepare(
                &f.b,
                &request.id,
                None,
                DraftInput {
                    action: Some(settings.action(&request.request)),
                    ..Default::default()
                },
            )
            .unwrap();
        let card = f.store.channel_action(&delivery).unwrap().unwrap();
        let summary = f.store.notification_summary(&delivery).unwrap();
        assert!(card.body.contains("Ask: Find the Lens performance PRs."));
        assert!(card.body.contains("From: Theo"));
        assert!(
            card.body.contains(
                "Where: [Slack thread](https://example.slack.com/archives/C1/p1000000001)"
            )
        );
        assert!(
            card.body.contains("answer and requested files")
                && card.body.contains("everyone in the source Slack channel")
        );
        assert!(!card.body.contains("private context dump"));
        f.store
            .channel_decide(&slack_actor(), &card.token, decision.clone())
            .unwrap();
        assert!(f.store.channel_action(&delivery).unwrap().is_none());
        assert_eq!(summary, f.store.notification_summary(&delivery).unwrap());
        if decision == Decision::Approve {
            let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
            f.store
                .stage_result(
                    &f.bw,
                    &request.id,
                    &work.lease,
                    DraftInput {
                        text: "The PRs are merged.".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(summary, f.store.notification_summary(&delivery).unwrap());
            assert!(f.store.channel_action(&delivery).unwrap().is_none());
        }
    }
}
