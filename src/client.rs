use crate::{config::ClientConfig, crypto::id, model::*};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::Method;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

#[derive(Clone)]
pub struct Client {
    pub config: ClientConfig,
    http: reqwest::Client,
    uploads: reqwest::Client,
}
impl Client {
    pub fn new(config: ClientConfig) -> Result<Self> {
        crate::config::validate_url(&config.relay, true)?;
        Ok(Self {
            config,
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(20))
                .read_timeout(std::time::Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            // Reqwest's read timeout also runs while sending the request body.
            // File uploads use their work heartbeat and cancellation instead.
            uploads: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    /// Revoke only this connection, using the existing logout endpoint.
    pub async fn disconnect(&self) -> Result<()> {
        let response = self
            .http
            .post(format!(
                "{}/auth/logout",
                self.config.relay.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context(
                "Cannot disconnect from the server. Reconnect to the internet and rerun uninstall.",
            )?;
        if response.status().is_success() || response.status() == reqwest::StatusCode::UNAUTHORIZED
        {
            return Ok(());
        }
        bail!(
            "The server could not disconnect this agent (HTTP {}). Rerun uninstall after checking the server.",
            response.status()
        );
    }
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T> {
        // Never interpret a tool-supplied identifier as an alternate API path.
        let parsed = url::Url::parse(&format!(
            "{}{}",
            self.config.relay.trim_end_matches('/'),
            path
        ))?;
        if !path.starts_with("/api/")
            || path.contains("..")
            || path.contains('\\')
            || parsed.origin() != url::Url::parse(&self.config.relay)?.origin()
        {
            bail!("Invalid Litecom API path.");
        }
        let mut request = self
            .http
            .request(
                method,
                format!("{}{}", self.config.relay.trim_end_matches('/'), path),
            )
            .bearer_auth(&self.config.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.context(
            "Cannot reach your Litecom server. Your saved requests will wait until it reconnects.",
        )?;
        let status = response.status();
        let value: Value = response
            .json()
            .await
            .context("The server did not return a Litecom response.")?;
        if !status.is_success() {
            return Err(ApiError {
                status: status.as_u16(),
                message: value["error"]["message"]
                    .as_str()
                    .unwrap_or("The server could not finish the request.")
                    .to_owned(),
            }
            .into());
        }
        Ok(serde_json::from_value(value)?)
    }
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.call(Method::GET, path, None).await
    }
    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: impl Serialize) -> Result<T> {
        self.call(Method::POST, path, Some(serde_json::to_value(body)?))
            .await
    }
    pub async fn ask(
        &self,
        contact: &str,
        agent: Option<&str>,
        request: &str,
        key: &str,
    ) -> Result<TaskView> {
        let target: Value = self
            .post("/api/resolve", json!({"contact":contact,"agent":agent}))
            .await?;
        let owner = target["owner_id"]
            .as_str()
            .context("The contact is missing an account ID.")?;
        let target_agent = target["agent_id"]
            .as_str()
            .context("The contact is missing an agent ID.")?;
        uuid::Uuid::parse_str(owner)?;
        uuid::Uuid::parse_str(target_agent)?;
        let existing: Option<TaskView> = self.post("/api/tasks/lookup", json!({"key":key})).await?;
        if let Some(existing) = existing {
            if existing.owner_id != owner
                || existing.request != request.trim()
                || (agent.is_some() && existing.agent_id != target_agent)
            {
                bail!("This request ID was already used for different work.");
            }
            return Ok(existing);
        }
        let response=self.http.post(format!("{}/a2a/{owner}/{target_agent}",self.config.relay.trim_end_matches('/')))
            .bearer_auth(&self.config.token).json(&json!({"jsonrpc":"2.0","id":id(),"method":"SendMessage","params":{"message":{"messageId":key,"role":"ROLE_USER","parts":[{"text":request}]},"configuration":{"returnImmediately":true}}})).send().await?;
        let status = response.status();
        let body: Value = response.json().await?;
        if !status.is_success() || body.get("error").is_some() {
            bail!(
                "{}",
                body["error"]["message"]
                    .as_str()
                    .unwrap_or("The A2A request failed.")
            );
        }
        let task = body["result"]["task"]["id"]
            .as_str()
            .context("The agent did not return a task ID.")?;
        uuid::Uuid::parse_str(task)?;
        self.get(&format!("/api/tasks/{task}")).await
    }
    pub async fn inbox(
        &self,
        before_time: Option<i64>,
        before_id: Option<String>,
    ) -> Result<Vec<TaskView>> {
        let query = match (before_time, before_id) {
            (None, None) => String::new(),
            (Some(time), Some(id)) => {
                uuid::Uuid::parse_str(&id)
                    .context("Use the inbox cursor returned with the previous page.")?;
                format!("?before_time={time}&before_id={id}")
            }
            _ => bail!("Use both before_time and before_id to read older requests."),
        };
        self.get(&format!("/api/tasks{query}")).await
    }
    pub async fn upload(&self, task: &str, lease: Option<&str>, path: &Path) -> Result<Artifact> {
        let file = tokio::fs::File::open(path).await?;
        if !file.metadata().await?.is_file() {
            bail!("Choose a regular file.");
        }
        let name = path
            .file_name()
            .context("The file needs a name.")?
            .to_string_lossy()
            .into_owned();
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        let form = reqwest::multipart::Form::new()
            .text(
                "metadata",
                json!({"lease":lease,"name":name,"media_type":"application/octet-stream"})
                    .to_string(),
            )
            .part(
                "file",
                reqwest::multipart::Part::stream(body).file_name(name),
            );
        let response = self
            .uploads
            .post(format!(
                "{}/api/tasks/{task}/files",
                self.config.relay.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.token)
            .multipart(form)
            .send();
        tokio::pin!(response);
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
        let response = loop {
            tokio::select! {
                response = &mut response => break response?,
                _ = heartbeat.tick(), if lease.is_some() => {
                    let status: Status = self.post(&format!("/api/tasks/{task}/heartbeat"),json!({"lease":lease})).await?;
                    if !matches!(status,Status::Preparing|Status::Executing) { bail!("The upload was cancelled."); }
                }
            }
        };
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            bail!(
                "{}",
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("The file could not be uploaded.")
            );
        }
        Ok(serde_json::from_value(value)?)
    }
    pub async fn download(
        &self,
        task_id: &str,
        artifact_id: &str,
        directory: &Path,
    ) -> Result<PathBuf> {
        uuid::Uuid::parse_str(task_id).context("Use the request ID shown in your inbox.")?;
        uuid::Uuid::parse_str(artifact_id).context("Use a file ID from the approved response.")?;
        let task: TaskView = self.get(&format!("/api/tasks/{task_id}")).await?;
        let response = task
            .response
            .context("This response has not been shared yet.")?;
        let file = response
            .attachments
            .iter()
            .find(|f| f.id == artifact_id)
            .context("This file is not part of the approved response.")?;
        let response = self
            .http
            .get(format!(
                "{}/api/files/{artifact_id}",
                self.config.relay.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("This download is unavailable. Check the request for expiry or revocation.");
        }
        if !directory.exists() {
            crate::config::private_dir(directory)?;
        }
        if !directory.is_dir() {
            bail!("Choose a download folder.");
        }
        // On Windows canonicalization also supplies the extended-length path prefix.
        let directory = directory.canonicalize()?;
        let name = download_name(file);
        let path = directory.join(name);
        use std::io::Write;
        let mut output = tempfile::NamedTempFile::new_in(&directory)
            .context("Cannot create a file in this download folder.")?;
        let mut stream = response.bytes_stream();
        let mut digest = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            output.write_all(&chunk)?;
            digest.update(&chunk);
            size = size
                .checked_add(chunk.len() as u64)
                .context("File size cannot be represented on this platform.")?;
        }
        if size != file.size || hex::encode(digest.finalize()) != file.sha256 {
            bail!("Download integrity check failed. Retry the download.");
        }
        output.as_file().sync_all()?;
        output.persist_noclobber(&path).map_err(|error| {
            let message = if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                "A file with this name already exists. Choose another download folder."
            } else {
                "Cannot save the file in this download folder."
            };
            anyhow::Error::new(error.error).context(message)
        })?;
        let receipt: anyhow::Result<TaskView> = self
            .post(
                &format!("/api/files/{artifact_id}/received"),
                json!({"sha256":file.sha256}),
            )
            .await;
        if receipt.is_err() {
            tracing::warn!(
                "File saved and verified, but the server has not acknowledged its download receipt."
            );
        }
        Ok(path)
    }
    pub fn review_url(&self, task: &str) -> String {
        format!("{}/request/{task}", self.config.relay.trim_end_matches('/'))
    }
    pub fn request_id() -> String {
        id()
    }
}

pub(crate) fn download_name(file: &Artifact) -> String {
    // Sender filenames may come from a different OS. Keep the original in the
    // response metadata, but save a portable name within a 255-byte component.
    let clean: String = file
        .name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim_end_matches([' ', '.']);
    let (stem, extension) = clean
        .rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.len() <= 20)
        .map(|(stem, ext)| (stem, format!(".{ext}")))
        .unwrap_or((clean, String::new()));
    let mut stem = stem.to_owned();
    while stem.len() + extension.len() > 246 {
        stem.pop();
    }
    format!("{}-{stem}{extension}", &file.id[..8])
}
