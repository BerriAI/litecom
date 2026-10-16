use super::*;

#[test]
fn update_pause_drains_work_without_changing_approval_or_replaying() {
    let f = Fixture::new();
    let task = f.draft("running", Some(action()));
    f.approve(&task);
    let work = f.store.claim(&f.bw, &f.agent).unwrap().unwrap();
    let queued = f.request("queued");
    let update = uuid::Uuid::new_v4().to_string();
    assert_eq!(f.store.worker_update(&f.bw, &update, false).unwrap(), 1);
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    assert!(
        f.store
            .claim_read_capable(&f.bw, &f.agent)
            .unwrap()
            .is_none()
    );
    assert!(
        f.store
            .claim_preparation(&f.bw, &f.agent)
            .unwrap()
            .is_none()
    );
    f.store
        .finish(
            &f.bw,
            &task.id,
            &work.lease,
            Receipt {
                outcome: Outcome::Succeeded,
                summary: "Done".into(),
                urls: vec![],
            },
        )
        .unwrap();
    assert_eq!(f.store.worker_update(&f.bw, &update, false).unwrap(), 0);
    f.store.worker_update(&f.bw, &update, true).unwrap();
    assert_eq!(
        f.store.claim(&f.bw, &f.agent).unwrap().unwrap().task_id,
        queued.id
    );
}

#[test]
fn update_pause_is_bound_to_the_agent_credential_and_expires() {
    let f = Fixture::new();
    let update = uuid::Uuid::new_v4().to_string();
    assert!(f.store.worker_update(&f.a, &update, false).is_err());
    assert!(f.store.worker_update(&f.b, &update, false).is_err());
    assert!(f.store.worker_update(&f.bw, "bad", false).is_err());
    f.store.worker_update(&f.bw, &update, false).unwrap();
    let other = worker(&f.store, &f.b, &f.agent);
    assert!(f.store.worker_update(&other, &update, true).is_err());
    assert!(
        f.store
            .worker_update(&f.bw, &uuid::Uuid::new_v4().to_string(), true)
            .is_err()
    );
    let queued = f.request("waiting");
    f.advance(179);
    assert!(f.store.claim(&f.bw, &f.agent).unwrap().is_none());
    f.advance(2);
    assert_eq!(
        f.store.claim(&f.bw, &f.agent).unwrap().unwrap().task_id,
        queued.id
    );
}
