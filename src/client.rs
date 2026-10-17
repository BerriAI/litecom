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
    transfers: reqwest::Client,
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
            // Files and final replies use their work heartbeat and cancellation instead.
            transfers: reqwest::Client::builder()
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
        self.call_with(&self.http, method, path, body).await
    }
    async fn call_with<T: DeserializeOwned>(
        &self,
        http: &reqwest::Client,
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
        let mut request = http
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
        let value = response.json::<Value>().await;
        if !status.is_success() {
            return Err(ApiError {
                status: status.as_u16(),
                message: if status == reqwest::StatusCode::UNAUTHORIZED {
                    "This Litecom connection was disconnected. Run `litecom reconnect` to reconnect it; your agent settings are kept.".into()
                } else { value
                    .as_ref()
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!("The server could not finish the request (HTTP {status}).")
                    }) },
            }
            .into());
        }
        let value = value.context("The server did not return a Litecom response.")?;
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
        self.ask_with_access(contact, agent, request, key, false)
            .await
    }
    pub async fn ask_with_access(
        &self,
        contact: &str,
        agent: Option<&str>,
        request: &str,
        key: &str,
        ask_only: bool,
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
                || existing.ask_only != ask_only
                || existing.request != request.trim()
                || (agent.is_some() && existing.agent_id != target_agent)
            {
                bail!("This request ID was already used for different work.");
            }
            return Ok(existing);
        }
        if ask_only {
            // Older servers reject this unknown field. Never send ask-only as
            // optional A2A metadata to an unverified legacy server.
            return self
                .post(
                    "/api/tasks",
                    NewTask {
                        owner_id: owner.into(),
                        agent_id: Some(target_agent.into()),
                        request: request.into(),
                        idempotency_key: key.into(),
                        ask_only: true,
                    },
                )
                .await;
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
        let mut renewal = WorkLease::new(self, task, lease.unwrap_or(""));
        self.upload_with_lease(task, lease, path, &mut renewal)
            .await
    }
    pub(crate) async fn upload_with_lease(
        &self,
        task: &str,
        lease: Option<&str>,
        path: &Path,
        renewal: &mut WorkLease<'_>,
    ) -> Result<Artifact> {
        loop {
            if lease.is_some()
                && !matches!(
                    renewal.renew().await?,
                    Status::Preparing | Status::Executing
                )
            {
                bail!("The upload was cancelled.");
            }
            match self.upload_once(task, lease, path, renewal).await {
                Err(error) if lease.is_some() && transient(&error) => renewal.retry_delay().await?,
                result => return result,
            }
        }
    }
    async fn upload_once(
        &self,
        task: &str,
        lease: Option<&str>,
        path: &Path,
        renewal: &mut WorkLease<'_>,
    ) -> Result<Artifact> {
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
            .transfers
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
                    let status = renewal.renew().await?;
                    if !matches!(status,Status::Preparing|Status::Executing) { bail!("The upload was cancelled."); }
                }
            }
        };
        let status = response.status();
        let value = response.json::<Value>().await;
        if !status.is_success() {
            return Err(ApiError {
                status: status.as_u16(),
                message: value
                    .as_ref()
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str())
                    .unwrap_or("The file could not be uploaded.")
                    .to_owned(),
            }
            .into());
        }
        Ok(serde_json::from_value(value?)?)
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

/// Retry transport failures only while the server's existing execution lease
/// remains valid. Never acquire new work or restart an agent as part of retrying.
pub(crate) struct WorkLease<'a> {
    client: &'a Client,
    task: &'a str,
    lease: &'a str,
    deadline: tokio::time::Instant,
}
impl<'a> WorkLease<'a> {
    pub(crate) fn new(client: &'a Client, task: &'a str, lease: &'a str) -> Self {
        Self {
            client,
            task,
            lease,
            deadline: tokio::time::Instant::now()
                + std::time::Duration::from_secs(crate::store::LEASE_TTL as u64),
        }
    }
    async fn retry_delay(&self) -> Result<()> {
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + std::time::Duration::from_secs(2)).min(self.deadline),
        )
        .await;
        if tokio::time::Instant::now() >= self.deadline {
            bail!(
                "The execution lease expired while reconnecting. The work will not be repeated automatically."
            );
        }
        Ok(())
    }
    async fn submit_result_once(&mut self, draft: &DraftInput) -> Result<TaskView> {
        let client = self.client;
        let path = format!("/api/tasks/{}/result", self.task);
        let response = client.call_with(
            &client.transfers,
            Method::POST,
            &path,
            Some(json!({"lease":self.lease,"draft":draft})),
        );
        let keep_alive = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                if self.renew().await? != Status::Executing {
                    bail!("The request is no longer executing.");
                }
            }
        };
        tokio::select! {
            // A completed result can clear its lease before the HTTP response
            // arrives. Prefer the response; otherwise confirm it with a read.
            biased;
            result = response => result,
            result = keep_alive => result,
        }
    }
    /// Retry delivery of the already produced result; never repeat agent execution.
    pub(crate) async fn submit_result(&mut self, draft: DraftInput) -> Result<TaskView> {
        let path = format!("/api/tasks/{}", self.task);
        loop {
            let result = self.submit_result_once(&draft).await;
            match result {
                Ok(task) => return Ok(task),
                Err(error) => {
                    // The POST may have committed even if its response was lost.
                    // A read with matching content confirms delivery without a new lease.
                    loop {
                        // Reading an already committed reply needs no lease.
                        // Allow one bounded read even if renewal just expired.
                        match self.client.get::<TaskView>(&path).await {
                            Ok(task) => {
                                if matches!(
                                    task.status,
                                    Status::Completed | Status::AwaitingApproval
                                ) && task.revoked_at.is_none()
                                    && task
                                        .receipt
                                        .as_ref()
                                        .is_some_and(|r| r.outcome == Outcome::Succeeded)
                                    && task.draft.as_ref().is_some_and(|d| {
                                        d.text == draft.text
                                            && d.action.is_none()
                                            && d.attachments
                                                .iter()
                                                .map(|a| &a.id)
                                                .eq(draft.attachment_ids.iter())
                                    })
                                {
                                    return Ok(task);
                                }
                                if task.status != Status::Executing {
                                    return Err(error);
                                }
                                break;
                            }
                            Err(read_error) if transient(&read_error) => self.retry_delay().await?,
                            _ => return Err(error),
                        }
                    }
                    if !transient(&error) {
                        return Err(error);
                    }
                    self.retry_delay().await?;
                    if self.renew().await? != Status::Executing {
                        bail!("The request is no longer executing.");
                    }
                }
            }
        }
    }
    pub(crate) async fn renew(&mut self) -> Result<Status> {
        loop {
            let started = tokio::time::Instant::now();
            let attempt = tokio::time::timeout_at(
                self.deadline,
                self.client.post::<Status>(
                    &format!("/api/tasks/{}/heartbeat", self.task),
                    json!({"lease":self.lease}),
                ),
            )
            .await;
            match attempt {
                Ok(Ok(status)) => {
                    // Count from request start, not response receipt: a slow
                    // response must not extend our view of server authority.
                    self.deadline =
                        started + std::time::Duration::from_secs(crate::store::LEASE_TTL as u64);
                    return Ok(status);
                }
                Ok(Err(error)) if transient(&error) => {
                    eprintln!(
                        "Relay temporarily unavailable; reconnecting request {}.",
                        self.task
                    );
                    tokio::time::sleep_until(
                        (tokio::time::Instant::now() + std::time::Duration::from_secs(2))
                            .min(self.deadline),
                    )
                    .await;
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => bail!(
                    "The execution lease expired while the relay was unavailable. The agent was stopped; it will not be restarted automatically."
                ),
            }
            if tokio::time::Instant::now() >= self.deadline {
                bail!(
                    "The execution lease expired while the relay was unavailable. The agent was stopped; it will not be restarted automatically."
                );
            }
        }
    }
}
pub(crate) fn transient(error: &anyhow::Error) -> bool {
    if let Some(api) = error.downcast_ref::<ApiError>() {
        return api.status == 408 || api.status == 429 || api.status >= 500;
    }
    error.downcast_ref::<reqwest::Error>().is_some_and(|e| {
        e.is_connect() || e.is_timeout() || e.is_body() || e.is_decode() || e.is_request()
    })
}

#[cfg(test)]
mod lease_tests {
    use super::*;
    #[tokio::test]
    async fn lease_rejection_is_immediate_and_transport_retry_stops_at_deadline() {
        use axum::{http::StatusCode, routing::post};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        for rejected in [true, false] {
            let count = Arc::new(AtomicUsize::new(0));
            let calls = count.clone();
            let router = axum::Router::new().route(
                "/api/tasks/test/heartbeat",
                post(move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        (
                            if rejected {
                                StatusCode::FORBIDDEN
                            } else {
                                StatusCode::SERVICE_UNAVAILABLE
                            },
                            "<html>unavailable</html>",
                        )
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let relay = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let client = Client::new(ClientConfig {
                relay,
                token: "fixture".into(),
                user_id: "fixture".into(),
                agent_id: "fixture".into(),
                workspace: None,
                runner: None,
                approval_channel: None,
            })
            .unwrap();
            let mut lease = WorkLease::new(&client, "test", "fixture");
            lease.deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(100);
            let error = lease.renew().await.unwrap_err();
            if rejected {
                assert_eq!(error.downcast_ref::<ApiError>().unwrap().status, 403);
            } else {
                assert!(error.to_string().contains("expired"));
            }
            assert_eq!(count.load(Ordering::SeqCst), 1);
            server.abort();
        }
    }
}
