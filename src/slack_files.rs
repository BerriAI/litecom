//! Deliver only attachments covered by the owner's approval, to the source thread.
use crate::{
    api::AppState,
    error::{Error, Result},
    model::{Artifact, Status},
    slack::SlackDestination,
    slack_agent::Job,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Delivery {
    file_id: String,
    upload_url: String,
    uploaded: bool,
    complete: bool,
    fallback: bool,
}

fn attachments(s: &AppState, job: &Job) -> Result<Vec<Artifact>> {
    let p = super::slack_agent::principal(s, job)?;
    let task = s
        .db()?
        .get_task(&p, job.task.as_deref().ok_or(Error::NotFound)?)?;
    if task.status != Status::Completed {
        return Ok(vec![]);
    }
    Ok(task.response.map(|r| r.attachments).unwrap_or_default())
}

// Process one persisted phase at a time. A retry reuses the same Slack file ID;
// it cannot create a second agent run or silently widen an old approval.
pub(crate) async fn advance(
    s: &AppState,
    job: &mut Job,
    connection: &SlackDestination,
) -> Result<bool> {
    if !job.share_files {
        return Ok(false);
    }
    let Some(artifact) = attachments(s, job)?.into_iter().find(|a| {
        !job.files
            .get(&a.id)
            .is_some_and(|d| d.complete || d.fallback)
    }) else {
        return Ok(false);
    };
    let mut delivery = job.files.get(&artifact.id).cloned().unwrap_or_default();
    let result = transfer(s, job, connection, &artifact, &mut delivery).await;
    if let Err(error) = result {
        // Keep the completed result usable when Slack rejects a file (missing
        // scope, workspace policy, size, etc.) or delivery cannot be confirmed.
        tracing::warn!(%error, "Slack file delivery unavailable; keeping authenticated download");
        delivery.fallback = true;
    }
    job.files.insert(artifact.id, delivery);
    s.db()?.checkpoint_slack(job)?;
    Ok(true)
}

async fn transfer(
    s: &AppState,
    job: &Job,
    connection: &SlackDestination,
    artifact: &Artifact,
    delivery: &mut Delivery,
) -> Result<()> {
    let slack = s.providers.slack.as_ref().ok_or(Error::Unauthorized)?;
    let p = super::slack_agent::principal(s, job)?;
    s.db()?.download_metadata(&p, &artifact.id)?;
    if delivery.file_id.is_empty() {
        (delivery.file_id, delivery.upload_url) = slack
            .upload_ticket(s.providers.http(), connection, artifact)
            .await?;
    } else if !delivery.uploaded {
        let state = s.clone();
        let checkpoint = job.clone();
        let id = artifact.id.clone();
        let expected = artifact.clone();
        let body = futures_util::stream::try_unfold(
            (0u64, 0u64, Sha256::new()),
            move |(sequence, size, mut digest)| {
                let state = state.clone();
                let checkpoint = checkpoint.clone();
                let p = p.clone();
                let id = id.clone();
                let expected = expected.clone();
                async move {
                    let db = state.db().map_err(std::io::Error::other)?;
                    db.checkpoint_slack(&checkpoint)
                        .map_err(std::io::Error::other)?;
                    let bytes = db
                        .download_chunk(&p, &id, sequence)
                        .map_err(std::io::Error::other)?;
                    let Some(bytes) = bytes else {
                        if size != expected.size
                            || hex::encode(digest.finalize()) != expected.sha256
                        {
                            return Err(std::io::Error::other(
                                "The file failed its integrity check.",
                            ));
                        }
                        return Ok(None);
                    };
                    let size = size
                        .checked_add(bytes.len() as u64)
                        .ok_or_else(|| std::io::Error::other("Invalid file size."))?;
                    digest.update(&bytes);
                    if size > expected.size
                        || (size == expected.size
                            && hex::encode(digest.clone().finalize()) != expected.sha256)
                    {
                        return Err(std::io::Error::other(
                            "The file failed its integrity check.",
                        ));
                    }
                    Ok::<_, std::io::Error>(Some((bytes, (sequence + 1, size, digest))))
                }
            },
        );
        // No bot token goes to the signed upload URL. Do not follow redirects
        // or impose a whole-file timeout/size limit on streaming uploads.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Internal(e.into()))?;
        let response = http
            .post(&delivery.upload_url)
            .header(reqwest::header::CONTENT_LENGTH, artifact.size)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(reqwest::Body::wrap_stream(body))
            .send()
            .await
            .map_err(|_| Error::Unavailable("Slack could not receive the file.".into()))?;
        if !response.status().is_success() {
            return Err(Error::Unavailable(
                "Slack did not accept the file upload.".into(),
            ));
        }
        delivery.uploaded = true;
    } else {
        // Recheck authorization immediately before sharing with the channel.
        s.db()?.download_metadata(&p, &artifact.id)?;
        slack
            .share_file(
                s.providers.http(),
                connection,
                &delivery.file_id,
                &job.mention.channel,
                &job.mention.thread_ts,
            )
            .await?;
        delivery.complete = true;
        delivery.upload_url.clear();
    }
    Ok(())
}

pub(crate) fn download_buttons(s: &AppState, job: &Job) -> Result<Vec<Value>> {
    if attachments(s, job)?
        .iter()
        .any(|a| !job.files.get(&a.id).is_some_and(|d| d.complete))
    {
        return Ok(vec![crate::slack::view_button(
            "Download files",
            &format!(
                "{}/request/{}",
                s.origin,
                job.task.as_deref().ok_or(Error::NotFound)?
            ),
        )]);
    }
    Ok(vec![])
}

pub(crate) fn has_fallback(job: &Job) -> bool {
    job.files.values().any(|d| d.fallback)
}
