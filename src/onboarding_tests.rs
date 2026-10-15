use super::*;
use crate::google::Identity;

fn google_user(store: &Store, subject: &str) -> (Principal, Session) {
    let profile = store
        .google_profile(&Identity {
            subject: subject.into(),
            email: "moe@example.com".into(),
            name: "Moe".into(),
            audience: "test-client".into(),
            domain: Some("example.com".into()),
        })
        .unwrap();
    let session = store.session_for(profile).unwrap();
    (
        store.authenticate(&session.token, Role::Human).unwrap(),
        session,
    )
}

#[test]
fn google_identity_is_stable_without_a_phone_and_does_not_merge_by_email() {
    let f = Fixture::new();
    let (p, s) = google_user(&f.store, "subject");
    assert!(!f.store.has_phone(&p).unwrap());
    assert!(f.store.enable_phone(&p).is_err());
    let (_, again) = google_user(&f.store, "subject");
    assert_eq!(s.profile.id, again.profile.id);
    let (_, different) = google_user(&f.store, "other-subject");
    assert_ne!(s.profile.id, different.profile.id);
    f.store.connect_google_email(&p).unwrap();
    assert!(
        f.store
            .channels(&p)
            .unwrap()
            .iter()
            .any(|c| c.kind == "email" && c.verified)
    );
    assert!(f.store.google_identity(&f.bw).is_err());
}

#[test]
fn google_login_state_is_browser_bound_expiring_and_one_use() {
    let f = Fixture::new();
    f.store
        .save_login_state("state", "browser", &"private nonce")
        .unwrap();
    assert!(
        f.store
            .take_login_state::<String>("state", "other")
            .is_err()
    );
    assert_eq!(
        f.store
            .take_login_state::<String>("state", "browser")
            .unwrap(),
        "private nonce"
    );
    assert!(
        f.store
            .take_login_state::<String>("state", "browser")
            .is_err()
    );
    f.store
        .save_login_state("expired", "browser", &"nonce")
        .unwrap();
    f.advance(601);
    assert!(
        f.store
            .take_login_state::<String>("expired", "browser")
            .is_err()
    );
}

#[test]
fn google_phone_link_is_purpose_bound_and_cannot_take_another_account() {
    let f = Fixture::new();
    let (p, _) = google_user(&f.store, "phone-owner");
    let (challenge, code) = f.store.begin_phone(&p, "+14155550999").unwrap();
    assert!(f.store.verify_email(&p, &challenge.id, &code).is_err());
    assert!(f.store.verify_phone(&f.b, &challenge.id, &code).is_err());
    f.store.verify_phone(&p, &challenge.id, &code).unwrap();
    assert!(f.store.verify_phone(&p, &challenge.id, &code).is_err());
    assert!(f.store.has_phone(&p).unwrap());
    assert_eq!(f.store.phone(&p.user_id).unwrap(), "+14155550999");
    let (other, _) = google_user(&f.store, "different-owner");
    f.advance(61);
    let (challenge, code) = f.store.begin_phone(&other, "+14155550999").unwrap();
    assert!(f.store.verify_phone(&other, &challenge.id, &code).is_err());
    assert!(!f.store.has_phone(&other).unwrap());
    assert_eq!(f.store.phone(&p.user_id).unwrap(), "+14155550999");
}

#[test]
fn directory_preserves_private_fields_blocking_and_contact_approval() {
    let f = Fixture::new();
    let (p, _) = google_user(&f.store, "newcomer");
    let people = f.store.people(&p).unwrap();
    assert_eq!(people.len(), 2);
    let json = serde_json::to_string(&people).unwrap();
    assert!(!json.contains("phone") && !json.contains("invite") && !json.contains("token"));
    assert!(f.store.people(&f.bw).is_err());
    let request = || NewTask {
        owner_id: f.b.user_id.clone(),
        agent_id: None,
        request: "hello".into(),
        idempotency_key: "directory".into(),
    };
    assert!(f.store.create_task(&p, request()).is_err());
    f.store.request_contact_to(&p, &f.b.user_id).unwrap();
    f.store.request_contact_to(&p, &f.b.user_id).unwrap();
    assert!(f.store.create_task(&p, request()).is_err());
    f.store.accept_contact(&f.b, &p.user_id).unwrap();
    assert!(f.store.create_task(&p, request()).is_ok());
    f.store.block_contact(&f.b, &p.user_id).unwrap();
    assert!(
        !f.store
            .people(&p)
            .unwrap()
            .iter()
            .any(|c| c.id == f.b.user_id)
    );
}

#[test]
fn local_connection_is_owner_bound_and_waits_for_its_own_worker() {
    let f = Fixture::new();
    assert!(f.store.begin_local_connection(&f.bw, &f.agent).is_err());
    assert!(f.store.begin_local_connection(&f.a, &f.agent).is_err());
    let code = f.store.begin_local_connection(&f.b, &f.agent).unwrap();
    assert_eq!(
        f.store
            .local_connection_status(&f.b, &code.user_code)
            .unwrap(),
        "waiting"
    );
    assert!(
        f.store
            .local_connection_status(&f.a, &code.user_code)
            .is_err()
    );
    let token = f.store.poll_device(&code.device_code).unwrap().unwrap();
    assert!(f.store.poll_device(&code.device_code).is_err());
    let p = f.store.authenticate(&token.token, Role::Agent).unwrap();
    assert!(f.store.begin_local_connection(&p, &f.agent).is_err());
    f.store.local_connection_ready(&p).unwrap();
    f.store.claim(&f.bw, &f.agent).unwrap();
    assert_eq!(
        f.store
            .local_connection_status(&f.b, &code.user_code)
            .unwrap(),
        "offline"
    );
    f.store.claim(&p, &f.agent).unwrap();
    assert_eq!(
        f.store
            .local_connection_status(&f.b, &code.user_code)
            .unwrap(),
        "connected"
    );
    f.store.revoke_connection(&f.b, &p.credential_id).unwrap();
    assert_eq!(
        f.store
            .local_connection_status(&f.b, &code.user_code)
            .unwrap(),
        "disconnected"
    );
    let expired = f.store.begin_local_connection(&f.b, &f.agent).unwrap();
    f.advance(601);
    assert!(f.store.poll_device(&expired.device_code).is_err());
    assert_eq!(
        f.store
            .local_connection_status(&f.b, &expired.user_code)
            .unwrap(),
        "expired"
    );
}

#[test]
fn local_identity_preserves_custom_names_and_disconnect_is_not_a_heartbeat() {
    let f = Fixture::new();
    let identity = LocalAgentIdentity {
        device_name: "Moe’s MacBook Pro".into(),
        harness: "Codex".into(),
    };
    assert!(f.store.identify_local_agent(&f.b, &identity).is_err());
    f.store.identify_local_agent(&f.bw, &identity).unwrap();
    let agent = f.store.agents(&f.b, &f.b.user_id).unwrap().remove(0);
    assert_eq!(agent.name, identity.device_name);
    assert_eq!(agent.local.unwrap().harness, "Codex");
    assert!(!agent.connected);
    f.store.claim(&f.bw, &f.agent).unwrap();
    assert!(f.store.agents(&f.b, &f.b.user_id).unwrap()[0].connected);
    f.store
        .revoke_connection(&f.b, &f.bw.credential_id)
        .unwrap();
    let agent = f.store.agents(&f.b, &f.b.user_id).unwrap().remove(0);
    assert!(agent.last_seen_at.is_some());
    assert!(!agent.connected);

    let custom = f
        .store
        .register_agent(&f.b, "Work laptop", AgentKind::Local, false)
        .unwrap();
    let code = f.store.begin_local_connection(&f.b, &custom.id).unwrap();
    let token = f.store.poll_device(&code.device_code).unwrap().unwrap();
    let worker = f.store.authenticate(&token.token, Role::Agent).unwrap();
    f.store.identify_local_agent(&worker, &identity).unwrap();
    assert_eq!(
        f.store.owned_agent(&f.b, &custom.id).unwrap().name,
        "Work laptop"
    );
    assert!(f.store.owned_agent(&worker, &f.agent).is_err());
    let mut updated = identity;
    updated.harness = "Claude Code".into();
    f.store.identify_local_agent(&worker, &updated).unwrap();
    let agent = f
        .store
        .agents(&f.b, &f.b.user_id)
        .unwrap()
        .into_iter()
        .find(|a| a.id == custom.id)
        .unwrap();
    assert_eq!(agent.local.unwrap().harness, "Claude Code");
    assert_eq!(agent.name, "Work laptop");
    let people = serde_json::to_string(&f.store.people(&f.a).unwrap()).unwrap();
    assert!(!people.contains("example.com") && !people.contains("config"));
}

#[test]
fn v2_migration_preserves_accounts_connections_files_and_old_email_challenges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let (a, b, session, task, file, challenge, code) = {
        let store = Store::open(&path, [7; 32]).unwrap();
        let (a, _) = user(&store, "Theo", "+14155550201");
        let (b, session) = user(&store, "Moe", "+14155550202");
        store.request_contact(&a, &session.profile.invite).unwrap();
        store.accept_contact(&b, &a.user_id).unwrap();
        let task = store
            .create_task(
                &a,
                NewTask {
                    owner_id: b.user_id.clone(),
                    agent_id: None,
                    request: "note".into(),
                    idempotency_key: "migration".into(),
                },
            )
            .unwrap();
        let file = store
            .upload(&b, &task.id, None, "note.txt", "text/plain", b"preserved")
            .unwrap();
        let (challenge, code) = store.begin_email(&b, "moe@example.com").unwrap();
        store.db.execute_batch("PRAGMA foreign_keys=OFF; BEGIN; CREATE TABLE old_profiles(id TEXT PRIMARY KEY,name TEXT NOT NULL,phone_key TEXT NOT NULL UNIQUE,phone BLOB NOT NULL,invite_hash TEXT NOT NULL UNIQUE,invite BLOB NOT NULL); INSERT INTO old_profiles SELECT * FROM profiles; DROP TABLE profiles; ALTER TABLE old_profiles RENAME TO profiles; ALTER TABLE channel_challenges DROP COLUMN kind; PRAGMA user_version=2; COMMIT;").unwrap();
        (a, b, session, task, file, challenge, code)
    };
    let store = Store::open(&path, [7; 32]).unwrap();
    assert!(store.authenticate(&session.token, Role::Human).is_ok());
    assert!(
        store
            .contacts(&a)
            .unwrap()
            .iter()
            .any(|c| c.id == b.user_id && c.status == "accepted")
    );
    assert_eq!(store.get_task(&a, &task.id).unwrap().id, task.id);
    assert_eq!(store.download(&b, &file.id).unwrap().1, b"preserved");
    store.verify_email(&b, &challenge.id, &code).unwrap();
    let (new, _) = google_user(&store, "after-migration");
    assert!(!store.has_phone(&new).unwrap());
    assert!(
        !store
            .db
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}

#[tokio::test]
async fn google_auth_routes_are_public_but_browser_state_is_required() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let store = Store::open(":memory:", [7; 32]).unwrap();
    let mut providers = crate::providers::Providers::from_env(true).unwrap();
    providers.google = Some(crate::google::Google::new(
        "client".into(),
        "secret".into(),
        "example.com",
    ));
    let state = crate::api::AppState::new(store, "https://litecom.example".into(), providers);
    let app = crate::api::router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/google?next=https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("https://accounts.google.com/")
    );
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("HttpOnly; Secure")
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/google/callback?state=invalid&code=invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(!response.status().is_success());
}
