use super::*;
use crate::{backup, config};
use std::fs;

#[test]
fn live_backup_restores_data_without_reviving_access_or_running_actions() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let key = config::load_key(&source).unwrap();
    let store = Store::open(source.join("litecom.db"), key).unwrap();
    let (a, sa) = user(&store, "Theo", "+14155550411");
    let (b, sb) = user(&store, "Moe", "+14155550412");
    store.request_contact(&a, &sb.profile.invite).unwrap();
    store.accept_contact(&b, &a.user_id).unwrap();
    store.enable_phone(&a).unwrap();
    store.enable_phone(&b).unwrap();
    let agent = store.agents(&b, &b.user_id).unwrap()[0].id.clone();
    let device = store.begin_local_connection(&b, &agent).unwrap();
    let worker_token = store
        .poll_device(&device.device_code)
        .unwrap()
        .unwrap()
        .token;
    let bw = store.authenticate(&worker_token, Role::Agent).unwrap();
    let new_task = |key: &str| {
        store
            .create_task(
                &a,
                NewTask {
                    owner_id: b.user_id.clone(),
                    agent_id: None,
                    request: "Please send a note".into(),
                    idempotency_key: key.into(),
                },
            )
            .unwrap()
    };
    let shared = new_task("shared");
    let file = store
        .upload(
            &b,
            &shared.id,
            None,
            "note.txt",
            "text/plain",
            b"exact private note",
        )
        .unwrap();
    let draft = store
        .prepare(
            &b,
            &shared.id,
            None,
            DraftInput {
                text: "A note".into(),
                attachment_ids: vec![file.id.clone()],
                action: None,
            },
        )
        .unwrap()
        .draft
        .unwrap();
    store
        .decide(
            &b,
            &shared.id,
            draft.version,
            &draft.digest,
            Decision::Approve,
        )
        .unwrap();
    assert!(store.download(&a, &file.id).is_ok());
    let running = new_task("running");
    let work = store.claim(&bw, &agent).unwrap().unwrap();
    let draft = store
        .prepare(
            &bw,
            &running.id,
            Some(&work.lease),
            DraftInput {
                text: String::new(),
                attachment_ids: vec![],
                action: Some(action()),
            },
        )
        .unwrap()
        .draft
        .unwrap();
    store
        .decide(
            &b,
            &running.id,
            draft.version,
            &draft.digest,
            Decision::Approve,
        )
        .unwrap();
    store.claim(&bw, &agent).unwrap().unwrap();
    assert!(source.join("litecom.db-wal").exists());
    let snapshot = temp.path().join("snapshot");
    backup::create(&source, &snapshot).unwrap();
    backup::verify(&snapshot).unwrap();
    // Revocations after the snapshot must not silently be lost on restoration.
    store.block_contact(&b, &a.user_id).unwrap();
    let target = temp.path().join("recovered");
    backup::restore(&snapshot, &target).unwrap();
    assert!(!target.join(".restoring").exists());
    let restored = Store::open(
        target.join("litecom.db"),
        config::load_key(&target).unwrap(),
    )
    .unwrap();
    assert!(restored.authenticate(&sa.token, Role::Human).is_err());
    assert!(restored.authenticate(&sb.token, Role::Human).is_err());
    assert!(restored.invite_profile(&sb.profile.invite).is_err());
    let (fresh_a, new_sa) = user(&restored, "Theo", "+14155550411");
    let (fresh_b, new_sb) = user(&restored, "Moe", "+14155550412");
    assert_eq!(new_sa.profile.id, sa.profile.id);
    assert_eq!(new_sb.profile.id, sb.profile.id);
    assert_eq!(restored.contacts(&fresh_b).unwrap().len(), 1);
    assert!(
        restored
            .get_task(&fresh_a, &shared.id)
            .unwrap()
            .response
            .is_none()
    );
    assert!(restored.download(&fresh_a, &file.id).is_err());
    let stopped = restored.get_task(&fresh_b, &running.id).unwrap();
    assert_eq!(stopped.status, Status::Failed);
    assert_eq!(stopped.receipt.unwrap().outcome, Outcome::Uncertain);
    assert!(restored.authenticate(&worker_token, Role::Agent).is_err());
    let new_worker = worker(&restored, &fresh_b, &agent);
    assert!(restored.claim(&new_worker, &agent).unwrap().is_none());
    let bytes: Vec<u8> = restored
        .db
        .query_row(
            "SELECT data FROM artifact_chunks WHERE artifact_id=? AND sequence=0",
            [&file.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        restored
            .vault
            .decrypt(&bytes, &format!("artifact-chunk:{}:0", file.id))
            .unwrap(),
        b"exact private note"
    );
    assert_eq!(
        restored
            .db
            .query_row("SELECT count(*) FROM deliveries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let retry = restored
        .create_task(
            &fresh_b,
            NewTask {
                owner_id: fresh_b.user_id.clone(),
                agent_id: None,
                request: "Confirm the recovered agent works".into(),
                idempotency_key: "after-restore".into(),
            },
        )
        .unwrap();
    let work = restored.claim(&new_worker, &agent).unwrap().unwrap();
    assert_eq!(work.task_id, retry.id);
    let draft = restored
        .prepare(
            &new_worker,
            &retry.id,
            Some(&work.lease),
            DraftInput {
                text: "Recovered and connected".into(),
                attachment_ids: vec![],
                action: None,
            },
        )
        .unwrap()
        .draft
        .unwrap();
    let result = restored
        .decide(
            &fresh_b,
            &retry.id,
            draft.version,
            &draft.digest,
            Decision::Approve,
        )
        .unwrap();
    assert_eq!(result.response.unwrap().text, "Recovered and connected");
    // The original snapshot stays intact; restoration only edits the copy.
    backup::verify(&snapshot).unwrap();
}

#[test]
fn backup_rejects_existing_targets_incomplete_snapshots_and_mismatched_keys() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let key = config::load_key(&source).unwrap();
    let _store = Store::open(source.join("litecom.db"), key).unwrap();
    let snapshot = temp.path().join("snapshot");
    backup::create(&source, &snapshot).unwrap();
    assert!(backup::create(&source, &snapshot).is_err());
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("important"), b"preserve me").unwrap();
    assert!(backup::restore(&snapshot, &existing).is_err());
    assert_eq!(
        fs::read(existing.join("important")).unwrap(),
        b"preserve me"
    );
    fs::write(snapshot.join("storage.key"), [6; 32]).unwrap();
    assert!(backup::verify(&snapshot).is_err());
    let mut manifest: backup::Manifest =
        serde_json::from_slice(&fs::read(snapshot.join("manifest.json")).unwrap()).unwrap();
    manifest.key_sha256 = hash([6; 32]);
    fs::write(
        snapshot.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        backup::verify(&snapshot).is_err(),
        "A matching checksum does not prove the key decrypts the database"
    );
    fs::remove_file(snapshot.join("manifest.json")).unwrap();
    assert!(backup::restore(&snapshot, &temp.path().join("bad")).is_err());
    assert!(!temp.path().join("bad").exists());
    let interrupted = temp.path().join("interrupted");
    fs::create_dir(&interrupted).unwrap();
    fs::write(interrupted.join(".restoring"), b"incomplete").unwrap();
    assert!(config::load_key(&interrupted).is_err());
    assert!(!interrupted.join("storage.key").exists());
}

#[cfg(unix)]
#[test]
fn backup_files_are_private_and_symlink_sources_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let key = config::load_key(&source).unwrap();
    let _store = Store::open(source.join("litecom.db"), key).unwrap();
    let snapshot = temp.path().join("snapshot");
    backup::create(&source, &snapshot).unwrap();
    assert_eq!(
        fs::metadata(&snapshot).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in ["manifest.json", "litecom.db", "storage.key"] {
        assert_eq!(
            fs::metadata(snapshot.join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    fs::rename(snapshot.join("storage.key"), snapshot.join("real.key")).unwrap();
    symlink(snapshot.join("real.key"), snapshot.join("storage.key")).unwrap();
    assert!(backup::verify(&snapshot).is_err());
}
