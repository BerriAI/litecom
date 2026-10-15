//! Cloud agents run on the relay, independent of any connected laptop.
use crate::{
    api::AppState,
    devin::{Devin, Reply, Session},
    error::{Error, Result, validate},
    model::{DraftInput, Outcome, Status},
    store::CloudClaim,
};
use std::time::Duration;

pub async fn run(state: AppState) -> Result<()> {
    let devin = Devin::new()?;
    loop {
        if let Err(error) = tick(&state, &devin).await {
            // Provider errors deliberately omit response bodies and credential URLs.
            tracing::warn!(%error, "Cloud worker needs attention");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
pub(crate) async fn tick(state: &AppState, devin: &Devin) -> Result<bool> {
    state.db()?.queue_cloud_work()?;
    let claim = state.db()?.claim_cloud_job()?;
    let Some(mut claim) = claim else {
        return Ok(false);
    };
    match process(state, devin, &mut claim).await {
        Ok(()) => {}
        Err(error) => {
            // A session with a known ID can be inspected again; session creation
            // without a recorded ID is never retried, including after a timeout.
            if claim.job.session.is_none() {
                state.db()?.cloud_finish(&claim, Outcome::Uncertain,
                    "Devin may have started, but Litecom could not confirm its session. Check Devin for the Litecom request before creating another. This request will not be launched again.")?;
            } else {
                state.db()?.cloud_progress(&claim, &format!("{error} Litecom will keep checking this session. You can also open it in Devin."), true)?;
                state.db()?.release_cloud(&claim, false)?;
            }
        }
    }
    Ok(true)
}

async fn process(state: &AppState, devin: &Devin, claim: &mut CloudClaim) -> Result<()> {
    let status = state.db()?.renew_cloud(claim)?;
    if !matches!(status, Status::Executing | Status::CancelRequested) {
        // A result may have committed just before the relay stopped.
        return state.db()?.release_cloud(claim, true);
    }
    if claim.phase == "starting" && claim.job.session.is_none() {
        return state.db()?.cloud_finish(claim,Outcome::Uncertain,
            "The relay stopped while starting Devin. Check Devin for this Litecom request before sending it again. Litecom will not repeat the launch.");
    }
    if status == Status::CancelRequested {
        if let Some(session) = &claim.job.session {
            devin
                .terminate(&claim.job.connection, &session.session_id)
                .await?;
        }
        return state.db()?.cloud_finish(
            claim,
            Outcome::Cancelled,
            if claim.job.session.is_some() {
                "Devin confirmed termination. Changes already made may remain."
            } else {
                "Cancelled before Devin was started."
            },
        );
    }
    if claim.phase == "ready" {
        state
            .db()?
            .cloud_progress(claim, "Starting Devin. Your laptop can be offline.", false)?;
        let session = devin
            .start(
                &claim.job.connection,
                &claim.job.work.task_id,
                &claim.job.work.request,
            )
            .await?;
        validate(
            !session.session_id.is_empty() && session.session_id.len() <= 200,
            "Devin did not return a valid session ID.",
        )?;
        state.db()?.cloud_started(claim, session)?;
        state.db()?.cloud_progress(
            claim,
            "Devin is working. You can open the session to follow its progress.",
            false,
        )?;
        return state.db()?.release_cloud(claim, false);
    }
    let id = &claim
        .job
        .session
        .as_ref()
        .ok_or_else(|| Error::Invalid("Cloud session is missing.".into()))?
        .session_id;
    let session = devin.get(&claim.job.connection, id).await?;
    validate(
        session.session_id == *id,
        "Devin returned a different session. No result has been shared.",
    )?;
    state.db()?.cloud_started(claim, session.clone())?;
    if session.status_detail.as_deref() == Some("finished") || session.status == "exit" {
        return collect_result(state, devin, claim, &session).await;
    }
    if session.status == "error" {
        return state.db()?.cloud_finish(claim,Outcome::Failed,"Devin reported an error. Open its session to inspect the work; changes already made may remain.");
    }
    let message = match session.status_detail.as_deref() {
        Some("waiting_for_user") => "Devin needs your reply. Open the session to continue.",
        Some("waiting_for_approval") => {
            "Devin needs a permission decision. Open the session to review it."
        }
        _ if session.status == "suspended" => {
            "Devin is paused. Open the session to check its plan, permissions, or next step."
        }
        _ => "Devin is working. You can open the session to follow its progress.",
    };
    state.db()?.cloud_progress(
        claim,
        message,
        session.status == "suspended"
            || matches!(
                session.status_detail.as_deref(),
                Some("waiting_for_user" | "waiting_for_approval")
            ),
    )?;
    state.db()?.release_cloud(claim, false)
}

async fn collect_result(
    state: &AppState,
    devin: &Devin,
    claim: &CloudClaim,
    session: &Session,
) -> Result<()> {
    let reply = session
        .structured_output
        .clone()
        .and_then(|v| serde_json::from_value::<Reply>(v).ok());
    let Some(reply) = reply else {
        return state.db()?.cloud_finish(claim,Outcome::Failed,"Devin stopped without a complete reply for review. Open the session to inspect its work. No output has been shared.");
    };
    if reply.text.len() > 2_000_000
        || reply.files.len() > 100
        || (reply.text.trim().is_empty() && reply.files.is_empty())
    {
        return state.db()?.cloud_finish(claim,Outcome::Failed,"Devin's result is empty or exceeds Litecom's reply limits. Open the session to inspect it. No output has been shared.");
    }
    let available = if reply.files.is_empty() {
        vec![]
    } else {
        devin
            .attachments(&claim.job.connection, &session.session_id)
            .await?
    };
    let mut selected = vec![];
    for requested in &reply.files {
        let matches = available
            .iter()
            .filter(|a| {
                a.source == "devin" && (a.attachment_id == *requested || a.name == *requested)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1
            || selected
                .iter()
                .any(|a: &&crate::devin::Attachment| a.attachment_id == matches[0].attachment_id)
        {
            return state.db()?.cloud_finish(claim,Outcome::Failed,"Devin returned missing, ambiguous, or duplicate attachments. Open its session to inspect them. No output has been shared.");
        }
        selected.push(matches[0]);
    }
    let mut ids = vec![];
    for file in selected {
        if state.db()?.renew_cloud(claim)? != Status::Executing {
            return state.db()?.release_cloud(claim, false);
        }
        let bytes = tokio::time::timeout(
            Duration::from_secs(60),
            devin.download(&claim.job.connection, file),
        )
        .await
        .map_err(|_| Error::Unavailable("The attachment download timed out.".into()))??;
        let db = state.db()?;
        if db.renew_cloud(claim)? != Status::Executing {
            return db.release_cloud(claim, false);
        }
        let artifact = db.upload(
            &claim.job.principal,
            &claim.job.work.task_id,
            Some(&claim.job.work.lease),
            &file.name,
            file.content_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
            &bytes,
        )?;
        ids.push(artifact.id);
    }
    let db = state.db()?;
    if db.renew_cloud(claim)? != Status::Executing {
        return db.release_cloud(claim, false);
    }
    db.stage_result(
        &claim.job.principal,
        &claim.job.work.task_id,
        &claim.job.work.lease,
        DraftInput {
            text: reply.text,
            attachment_ids: ids,
            action: None,
        },
    )?;
    db.cloud_progress(
        claim,
        "Devin finished. Review the exact reply and files below before sharing them.",
        false,
    )?;
    db.release_cloud(claim, true)
}
