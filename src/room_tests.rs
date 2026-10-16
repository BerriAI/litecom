use super::*;
use crate::{
    api::{AppState, router},
    error::Error,
    providers::Providers,
    rooms::*,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn create(f: &Fixture) -> Channel {
    f.store
        .room_create(
            &f.a,
            &CreateChannel {
                members: vec![f.b.user_id.clone()],
                topic: "Installer work".into(),
                request_id: "create".into(),
            },
        )
        .unwrap()
}
fn join(store: &Store, p: &Principal, id: &str, key: &str) -> String {
    store
        .room_join(
            p,
            id,
            &JoinChannel {
                session_key: key.into(),
                label: "Existing chat".into(),
            },
        )
        .unwrap()
}
fn message(participant: &str, key: &str, text: &str) -> ChannelSync {
    ChannelSync {
        participant_id: participant.into(),
        message: Some(text.into()),
        message_id: Some(key.into()),
        ..Default::default()
    }
}

#[test]
fn rooms_enforce_membership_sender_identity_and_no_execution() {
    let f = Fixture::new();
    let room = create(&f);
    let (stranger, _) = user(&f.store, "Stranger", "+14155550300");
    assert!(
        f.store
            .room_list(&stranger, None, false)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        f.store.room_page(&stranger, &room.id, 0),
        Err(Error::NotFound)
    ));
    assert!(
        f.store
            .room_join(
                &stranger,
                &room.id,
                &JoinChannel {
                    session_key: "x".into(),
                    label: "".into()
                }
            )
            .is_err()
    );
    let a = join(&f.store, &f.a, &room.id, "a");
    let b = join(&f.store, &f.bw, &room.id, "b");
    assert!(
        f.store
            .room_sync(&f.bw, &room.id, &message(&a, "spoof", "x"))
            .is_err()
    );
    let other_credential = worker(&f.store, &f.b, &f.agent);
    assert!(
        f.store
            .room_sync(&other_credential, &room.id, &message(&b, "spoof", "x"))
            .is_err()
    );
    assert!(f.store.room_leave(&f.a, &room.id, &b).is_err());
    assert!(f.store.room_close(&f.b, &room.id).is_err());
    let page = f
        .store
        .room_sync(
            &f.bw,
            &room.id,
            &message(&b, "one", "I'll handle the installer"),
        )
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(
        page.participants
            .iter()
            .find(|p| p.id == b)
            .unwrap()
            .user_id,
        f.b.user_id
    );
    assert_eq!(
        f.store
            .db
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    f.store.block_contact(&f.a, &f.b.user_id).unwrap();
    assert!(f.store.room_page(&f.bw, &room.id, 0).is_err());
    assert!(f.store.room_list(&f.a, None, false).unwrap().is_empty());
}

#[test]
fn room_retries_cursors_and_separate_chats_never_lose_messages() {
    let f = Fixture::new();
    let room = create(&f);
    assert_eq!(create(&f).id, room.id);
    assert!(
        f.store
            .room_create(
                &f.a,
                &CreateChannel {
                    members: vec![f.b.user_id.clone()],
                    topic: "Different".into(),
                    request_id: "create".into()
                }
            )
            .is_err()
    );
    let a = join(&f.store, &f.a, &room.id, "a");
    assert_eq!(a, join(&f.store, &f.a, &room.id, "a"));
    let b = join(&f.store, &f.a, &room.id, "another-chat");
    assert_ne!(a, b);
    let input = message(&a, "one", "Private coordination note");
    f.store.room_sync(&f.a, &room.id, &input).unwrap();
    assert_eq!(
        f.store
            .room_sync(&f.a, &room.id, &input)
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(
        f.store
            .room_sync(&f.a, &room.id, &message(&a, "one", "Changed"))
            .is_err()
    );
    let read = ChannelSync {
        participant_id: b,
        ..Default::default()
    };
    assert_eq!(
        f.store
            .room_sync(&f.a, &room.id, &read)
            .unwrap()
            .messages
            .len(),
        1
    );
    // More than one page: lost responses and another chat's reads do not ack it.
    for index in 2..=105 {
        f.store
            .room_sync(&f.a, &room.id, &message(&a, &format!("m{index}"), "Update"))
            .unwrap();
    }
    let page = f.store.room_page(&f.a, &room.id, 0).unwrap();
    assert!(page.has_more);
    assert_eq!(page.next_cursor, 100);
    assert_eq!(
        f.store.room_page(&f.a, &room.id, 0).unwrap().messages.len(),
        100
    );
    let end = f.store.room_page(&f.a, &room.id, page.next_cursor).unwrap();
    assert_eq!(end.messages.len(), 5);
    assert!(!end.has_more);
    assert_eq!(end.messages[0].sequence, 101);
    assert!(f.store.room_page(&f.a, &room.id, -1).is_err());
    f.store.room_close(&f.a, &room.id).unwrap();
    assert!(
        f.store
            .room_sync(&f.a, &room.id, &message(&a, "new", "Blocked"))
            .is_err()
    );
    // A previously accepted send can still be retried after close.
    assert!(f.store.room_sync(&f.a, &room.id, &input).is_ok());
    f.store.room_leave(&f.a, &room.id, &a).unwrap();
    assert!(f.store.room_sync(&f.a, &room.id, &input).is_err());
}

#[test]
fn rooms_survive_upgrade_restart_and_encrypt_content_but_quarantine_restore() {
    let f = Fixture::new();
    let room = create(&f);
    let participant = join(&f.store, &f.a, &room.id, "chat");
    let text = "Private content not plaintext in SQLite";
    f.store
        .room_sync(&f.a, &room.id, &message(&participant, "one", text))
        .unwrap();
    let data: Vec<u8> = f
        .store
        .db
        .query_row("SELECT text FROM room_messages", [], |r| r.get(0))
        .unwrap();
    assert!(
        !data
            .windows(text.len())
            .any(|bytes| bytes == text.as_bytes())
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rooms.db");
    f.store.db.backup(rusqlite::MAIN_DB, &path, None).unwrap();
    let reopened = Store::with_clock(&path, [7; 32], Arc::new(|| 1000)).unwrap();
    assert_eq!(
        reopened.room_page(&f.bw, &room.id, 0).unwrap().messages[0].text,
        text
    );
    reopened.quarantine_restored_state().unwrap();
    assert!(
        reopened
            .room_page(&f.a, &room.id, 0)
            .unwrap()
            .channel
            .closed
    );
    assert!(reopened.room_page(&f.bw, &room.id, 0).is_err());
    // Upgrading an existing v4 store is additive and preserves sign-in/account data.
    reopened.db.execute_batch("PRAGMA user_version=4; DROP TABLE room_messages; DROP TABLE room_participants; DROP TABLE room_members; DROP TABLE rooms;").unwrap();
    drop(reopened);
    let upgraded = Store::with_clock(path, [7; 32], Arc::new(|| 1000)).unwrap();
    assert!(upgraded.room_list(&f.a, None, false).unwrap().is_empty());
    assert_eq!(upgraded.profile(&f.a).unwrap().id, f.a.user_id);
}

#[test]
fn room_discovery_handles_partial_names_and_ambiguity() {
    let f = Fixture::new();
    let room = create(&f);
    assert_eq!(
        f.store.room_list(&f.a, Some("Moe"), false).unwrap()[0].id,
        room.id
    );
    user(&f.store, "Tin One", "+14155550301");
    let (tin, _) = user(&f.store, "Tin Two", "+14155550302");
    let mut input = CreateChannel {
        members: vec!["Tin".into()],
        topic: "".into(),
        request_id: "tin".into(),
    };
    assert!(matches!(
        f.store.room_create(&f.a, &input),
        Err(Error::Conflict(_))
    ));
    input.members = vec![tin.user_id.clone()];
    let channel = f.store.room_create(&f.a, &input).unwrap();
    assert!(channel.members.iter().any(|m| m.id == tin.user_id));
    f.store.room_close(&f.a, &channel.id).unwrap();
    assert_eq!(f.store.room_list(&f.a, None, false).unwrap().len(), 1);
    assert_eq!(f.store.room_list(&f.a, None, true).unwrap().len(), 2);
}

#[tokio::test]
async fn room_wait_releases_lock_returns_on_message_and_rechecks_revocation() {
    let f = Fixture::new();
    let room = create(&f);
    let device = f.store.begin_device("waiting chat").unwrap();
    f.store
        .authorize_device(&f.b, &device.user_code, &f.agent)
        .unwrap();
    let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
    let p = f.store.authenticate(&token.token, Role::Agent).unwrap();
    let participant = join(&f.store, &p, &room.id, "wait");
    let sender = join(&f.store, &f.a, &room.id, "sender");
    let state = AppState::new(
        f.store,
        "http://localhost".into(),
        Providers::development().unwrap(),
    );
    let app = router(state.clone());
    let request = |after| {
        Request::post(format!("/api/rooms/{}/sync", room.id))
            .header("authorization", format!("Bearer {}", token.token))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&ChannelSync {
                    participant_id: participant.clone(),
                    after,
                    wait_seconds: 25,
                    ..Default::default()
                })
                .unwrap(),
            ))
            .unwrap()
    };
    let waiting = tokio::spawn(app.clone().oneshot(request(0)));
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    state
        .db()
        .unwrap()
        .room_sync(&f.a, &room.id, &message(&sender, "one", "Ping"))
        .unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: ChannelPage = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 100_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page.messages[0].text, "Ping");
    let waiting = tokio::spawn(app.oneshot(request(page.next_cursor)));
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    state
        .db()
        .unwrap()
        .revoke_connection(&f.b, &p.credential_id)
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn room_browser_mutations_require_csrf_and_strangers_cannot_read() {
    let f = Fixture::new();
    let room = create(&f);
    let (_, session) = user(&f.store, "Stranger", "+14155550303");
    let state = AppState::new(
        f.store,
        "http://localhost".into(),
        Providers::development().unwrap(),
    );
    let app = router(state);
    let read = app
        .clone()
        .oneshot(
            Request::get(format!("/api/rooms/{}", room.id))
                .header("cookie", format!("litecom_session={}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::NOT_FOUND);
    let no_csrf = app
        .oneshot(
            Request::post("/api/rooms")
                .header("cookie", format!("litecom_session={}", session.token))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
}
