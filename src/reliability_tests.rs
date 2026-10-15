//! Failure and race cases exercised through independent database connections and HTTP.
use super::*;

fn disk_fixture() -> (tempfile::TempDir, Fixture) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let c = clock.clone();
    let store = Store::with_clock(
        directory.path().join("litecom.db"),
        [7; 32],
        Arc::new(move || c.load(Ordering::SeqCst)),
    )
    .unwrap();
    let (a, _) = user(&store, "Theo", "+14155550101");
    let (b, session) = user(&store, "Moe", "+14155550102");
    store.request_contact(&a, &session.profile.invite).unwrap();
    store.accept_contact(&b, &a.user_id).unwrap();
    let agent = store.agents(&b, &b.user_id).unwrap()[0].id.clone();
    let bw = worker(&store, &b, &agent);
    (
        directory,
        Fixture {
            store,
            a,
            b,
            bw,
            agent,
            clock,
        },
    )
}

fn race<T: Send>(
    directory: &std::path::Path,
    count: usize,
    operation: impl Fn(Store, usize) -> T + Sync,
) -> Vec<T> {
    let stores: Vec<_> = (0..count)
        .map(|_| {
            Store::with_clock(directory.join("litecom.db"), [7; 32], Arc::new(|| 1000)).unwrap()
        })
        .collect();
    let barrier = std::sync::Barrier::new(count);
    std::thread::scope(|scope| {
        let handles: Vec<_> = stores
            .into_iter()
            .enumerate()
            .map(|(i, store)| {
                let barrier = &barrier;
                let operation = &operation;
                scope.spawn(move || {
                    barrier.wait();
                    operation(store, i)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    })
}

#[test]
fn independent_connections_cannot_duplicate_requests_claims_or_decisions() {
    let (directory, f) = disk_fixture();
    // Independent SQLite connections exercise locking beyond the relay's mutex.
    let requester = f.a.clone();
    let owner_id = f.b.user_id.clone();
    let results = race(directory.path(), 16, |store, _| {
        store
            .create_task(
                &requester,
                NewTask {
                    owner_id: owner_id.clone(),
                    agent_id: None,
                    request: "Concurrent retry".into(),
                    idempotency_key: "same-request".into(),
                },
            )
            .unwrap()
    });
    let task_id = results[0].id.clone();
    assert!(results.iter().all(|task| task.id == task_id));
    assert_eq!(f.store.tasks(&f.a, None).unwrap().len(), 1);
    let runner = f.bw.clone();
    let agent = f.agent.clone();
    let claims = race(directory.path(), 12, |store, _| {
        store.claim(&runner, &agent).unwrap()
    });
    let mut claims = claims.into_iter().flatten();
    let work = claims.next().unwrap();
    assert!(claims.next().is_none(), "Only one worker may own the work");
    let task = f
        .store
        .prepare(
            &f.bw,
            &task_id,
            Some(&work.lease),
            DraftInput {
                text: "Exact response".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let draft = task.draft.unwrap();
    let owner = f.b.clone();
    let decisions = race(directory.path(), 12, |store, i| {
        let decision = if i % 2 == 0 {
            Decision::Approve
        } else {
            Decision::Decline
        };
        store
            .decide(&owner, &task_id, draft.version, &draft.digest, decision)
            .map(|task| task.status)
    });
    let final_task = f.store.get_task(&f.a, &task_id).unwrap();
    assert!(matches!(
        final_task.status,
        Status::Completed | Status::Declined
    ));
    assert_eq!(decisions.iter().filter(|result| result.is_ok()).count(), 6);
    assert!(
        decisions
            .into_iter()
            .flatten()
            .all(|status| status == final_task.status)
    );
    let count: i64 = f
        .store
        .db
        .query_row(
            "SELECT COUNT(*) FROM approvals WHERE task_id=?",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        final_task.response.is_some(),
        final_task.status == Status::Completed
    );
}

#[test]
fn editing_and_approving_from_different_connections_never_releases_unreviewed_text() {
    let (directory, f) = disk_fixture();
    for round in 0..20 {
        let task = f.draft(&format!("edit-race-{round}"), None);
        let draft = task.draft.unwrap();
        let owner = f.b.clone();
        let task_id = task.id.clone();
        race(directory.path(), 8, |store, i| {
            if i % 2 == 0 {
                let _ = store.decide(
                    &owner,
                    &task_id,
                    draft.version,
                    &draft.digest,
                    Decision::Approve,
                );
            } else {
                let _ = store.prepare(
                    &owner,
                    &task_id,
                    None,
                    DraftInput {
                        text: format!("Unreviewed replacement {i}"),
                        ..Default::default()
                    },
                );
            }
        });
        let requester_view = f.store.get_task(&f.a, &task.id).unwrap();
        if requester_view.status == Status::Completed {
            assert_eq!(requester_view.response.unwrap().text, "A private draft");
        } else {
            assert_eq!(requester_view.status, Status::AwaitingApproval);
            assert!(requester_view.response.is_none());
        }
        assert!(requester_view.draft.is_none());
    }
}

#[tokio::test]
async fn downloads_publish_only_verified_bytes_with_portable_filenames() {
    let f = Fixture::new();
    let requester_agent = f.store.agents(&f.a, &f.a.user_id).unwrap()[0].id.clone();
    let device = f.store.begin_device("Download test").unwrap();
    f.store
        .authorize_device(&f.a, &device.user_code, &requester_agent)
        .unwrap();
    let token = f.store.poll_device(&device.device_code).unwrap().unwrap();
    let task = f.request("filenames");
    let names = [
        format!("{}.txt", "a".repeat(251)),
        format!("{}.txt", "é".repeat(125)),
        "report: Q4 + <draft>?*.txt".into(),
        "trailing dots... ".into(),
    ];
    let payload = b"The exact approved bytes.\n";
    let files: Vec<_> = names
        .iter()
        .map(|name| {
            f.store
                .upload(&f.b, &task.id, None, name, "text/plain", payload)
                .unwrap()
        })
        .collect();
    let draft = f
        .store
        .prepare(
            &f.b,
            &task.id,
            None,
            DraftInput {
                text: "Filename edge cases".into(),
                attachment_ids: files.iter().map(|file| file.id.clone()).collect(),
                action: None,
            },
        )
        .unwrap();
    f.approve(&draft);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = crate::api::AppState::new(
        f.store,
        origin.clone(),
        crate::providers::Providers::from_env(true).unwrap(),
    );
    let mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let transfer_mode = mode.clone();
    let router = crate::api::router(state.clone()).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let mode = transfer_mode.clone();
            async move {
                if request.method() == axum::http::Method::GET
                    && request.uri().path().starts_with("/api/files/")
                {
                    use axum::{
                        body::{Body, Bytes},
                        response::IntoResponse,
                    };
                    match mode.load(Ordering::SeqCst) {
                        0 => {
                            return Body::from_stream(futures_util::stream::iter([
                                Ok(Bytes::from_static(b"partial")),
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::ConnectionReset,
                                    "test interruption",
                                )),
                            ]))
                            .into_response();
                        }
                        1 => return Body::from("corrupted response").into_response(),
                        _ => {}
                    }
                }
                next.run(request).await
            }
        },
    ));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = crate::client::Client::new(crate::config::ClientConfig {
        relay: origin,
        token: token.token,
        user_id: f.a.user_id.clone(),
        agent_id: requester_agent,
        workspace: None,
        runner: None,
        approval_channel: None,
    })
    .unwrap();
    let output = tempfile::tempdir().unwrap();
    for fault in [0, 1] {
        mode.store(fault, Ordering::SeqCst);
        assert!(
            client
                .download(&task.id, &files[0].id, output.path())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_dir(output.path()).unwrap().count(),
            0,
            "No partial or corrupt file is published"
        );
        assert!(
            state
                .db()
                .unwrap()
                .get_task(&f.a, &task.id)
                .unwrap()
                .downloads
                .is_empty()
        );
    }
    mode.store(2, Ordering::SeqCst);
    let response = reqwest::Client::new()
        .get(format!("{}/api/files/{}", client.config.relay, files[2].id))
        .bearer_auth(&client.config.token)
        .send()
        .await
        .unwrap();
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert!(
        disposition.ends_with("filename*=UTF-8''report%3A%20Q4%20%2B%20%3Cdraft%3E%3F%2A.txt"),
        "{disposition}"
    );
    for file in files {
        let path = client
            .download(&task.id, &file.id, output.path())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.len() <= 255);
        assert!(!name.chars().any(|c| "<>:\"/\\|?*".contains(c)));
        assert!(!name.ends_with([' ', '.']));
        let count = std::fs::read_dir(output.path()).unwrap().count();
        assert!(
            client
                .download(&task.id, &file.id, output.path())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            payload,
            "A retry cannot replace an existing file"
        );
        assert_eq!(
            std::fs::read_dir(output.path()).unwrap().count(),
            count,
            "Failed publication cleans up the temporary file"
        );
    }
    assert_eq!(
        state
            .db()
            .unwrap()
            .get_task(&f.a, &task.id)
            .unwrap()
            .downloads
            .len(),
        names.len()
    );
    server.abort();
}
