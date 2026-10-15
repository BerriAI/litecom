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
use base64::Engine;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
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
    assert!(p.body.contains("Theo asks:") && p.body.contains(&task.request));
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

fn sms(body: &str, sender: &str) -> (String, String) {
    let mut params = vec![
        ("AccountSid", "ACtest"),
        ("Body", body),
        ("From", sender),
        ("FutureParameter", "included"),
    ];
    params.sort();
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"twilio-secret").unwrap();
    mac.update(b"https://relay.example/webhooks/twilio");
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        mac.update(key.as_bytes());
        mac.update(value.as_bytes());
        form.append_pair(key, value);
    }
    (
        form.finish(),
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()),
    )
}
#[tokio::test]
async fn phone_callback_needs_signature_sender_and_exact_code() {
    let f = Fixture::new();
    let mut d = destination(&f, "phone");
    let task = f.draft("phone-http", Some(action()));
    d.task_id = task.id.clone();
    let a = f.store.channel_action(&d).unwrap().unwrap();
    let state = AppState::new(
        f.store,
        "https://relay.example".into(),
        Providers::test_replies("ACtest", "twilio-secret", "slack-secret"),
    );
    let app = router(state.clone());
    for (command, sender, signature_ok, expected) in [
        (
            format!("APPROVE {}", a.code),
            "+14155550102",
            false,
            StatusCode::UNAUTHORIZED,
        ),
        (
            format!("APPROVE {}", a.code),
            "+14155550101",
            true,
            StatusCode::OK,
        ),
        ("Approve".into(), "+14155550102", true, StatusCode::OK),
    ] {
        let (body, signature) = sms(&command, sender);
        let response = app
            .clone()
            .oneshot(
                Request::post("/webhooks/twilio")
                    .header(
                        "x-twilio-signature",
                        if signature_ok {
                            signature.as_str()
                        } else {
                            "fake"
                        },
                    )
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(count(&state.db().unwrap(), &task.id), 0);
    }
    let (body, signature) = sms(&format!("APPROVE {}", a.code), "+14155550102");
    let response = app
        .clone()
        .oneshot(
            Request::post("/webhooks/twilio")
                .header("x-twilio-signature", signature)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
            .contains("Approved.")
    );
    assert_eq!(count(&state.db().unwrap(), &task.id), 1);
    let (body, signature) = sms("STOP", "+14155550102");
    app.oneshot(
        Request::post("/webhooks/twilio")
            .header("x-twilio-signature", signature)
            .body(Body::from(body))
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(!state.db().unwrap().channels(&f.b).unwrap()[0].verified);
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
        Providers::test_replies("ACtest", "twilio-secret", "slack-secret"),
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

#[test]
fn twilio_signature_matches_published_reference_vector() {
    // https://www.twilio.com/docs/usage/security#explore-the-algorithm-yourself
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "x-twilio-signature",
        "L/OH5YylLD5NRKLltdqwSvS0BnU=".parse().unwrap(),
    );
    let params = [
        ("Digits", "1234"),
        ("To", "+18005551212"),
        ("From", "+14158675310"),
        ("Caller", "+14158675310"),
        ("CallSid", "CA1234567890ABCDE"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect::<Vec<_>>();
    crate::channel_replies::verify_twilio(
        &headers,
        &params,
        "https://example.com/myapp.php?foo=1&bar=2",
        "12345",
    )
    .unwrap();
    assert!(
        crate::channel_replies::verify_twilio(
            &headers,
            &params,
            "https://attacker.example/myapp.php?foo=1&bar=2",
            "12345"
        )
        .is_err()
    );
}
