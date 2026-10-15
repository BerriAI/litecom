//! Devin v3 transport. Provider credentials and unapproved results remain private.
use crate::{
    crypto::id,
    error::{Error, Result, validate},
    model::{Action, AgentKind},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub token: String,
    pub org_id: String,
    pub principal_id: String,
    pub principal_name: String,
    pub revision: String,
}
impl Connection {
    pub fn action(&self, request: &str) -> Action {
        Action::Cloud {
            title: "Run Devin for this request".into(),
            description: format!(
                "Start a cloud session in {} using this Devin account's existing repositories, context, tools, and default security profile. It can make changes within that access. Your Devin plan and its normal charges apply. Review the reply and files before Litecom shares them.",
                self.org_id
            ),
            instruction: request.into(),
            provider: AgentKind::Devin,
            acting_account: format!(
                "{} ({}) · {}",
                self.principal_name, self.principal_id, self.org_id
            ),
            connection_revision: self.revision.clone(),
        }
    }
}
#[derive(Clone)]
pub struct Devin {
    http: reqwest::Client,
    api: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub session_id: String,
    pub url: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub structured_output: Option<Value>,
}
#[derive(Clone, Deserialize)]
pub struct Attachment {
    pub attachment_id: String,
    pub name: String,
    pub content_type: Option<String>,
    pub source: String,
}
#[derive(Deserialize)]
pub struct Reply {
    pub text: String,
    #[serde(default)]
    pub files: Vec<String>,
}
pub(crate) fn owner_session_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("app.devin.ai")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443))
    .then(|| url.to_string())
}
fn public_download_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_download_ip(IpAddr::V4(v4));
            }
            let s = ip.segments();
            // Global unicast only; exclude documentation and tunnelling ranges.
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] == 0))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

async fn signed_download(url: url::Url) -> Result<reqwest::Response> {
    let host = url.host_str().unwrap_or("");
    validate(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && (host == "api.devin.ai" || host.ends_with(".amazonaws.com")),
        "Devin returned an unsupported download host.",
    )?;
    let addresses = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| Error::Unavailable("The download host could not be resolved.".into()))?
        .collect::<Vec<_>>();
    validate(
        !addresses.is_empty() && addresses.iter().all(|a| public_download_ip(a.ip())),
        "Devin returned a non-public download address.",
    )?;
    // Pin the checked DNS result and do not forward credentials or proxy settings
    // to signed file URLs. Redirects are checked separately on every hop.
    let http = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| Error::Internal(e.into()))?;
    http.get(url)
        .send()
        .await
        .map_err(|_| Error::Unavailable("The signed attachment download failed.".into()))
}
impl Devin {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(20))
                .read_timeout(std::time::Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| Error::Internal(e.into()))?,
            api: "https://api.devin.ai".into(),
        })
    }
    #[cfg(test)]
    pub(crate) fn test_api(api: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            api,
        }
    }
    async fn json(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request.send().await.map_err(|_| {
            Error::Unavailable(
                "Devin could not be reached. Your approved work will not be started twice.".into(),
            )
        })?;
        match response.status().as_u16() {
            401|403=>return Err(Error::Unavailable("Devin did not authorize this connection. Check its token and organization permissions.".into())),
            200..=299=>{},
            _=>return Err(Error::Unavailable("Devin did not complete the request. Check your plan, permissions, and service status.".into())),
        }
        response
            .json()
            .await
            .map_err(|_| Error::Unavailable("Devin returned an incomplete response.".into()))
    }
    fn part(value: &str) -> String {
        url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
    }
    fn org(&self, c: &Connection) -> String {
        format!("{}/v3/organizations/{}", self.api, Self::part(&c.org_id))
    }
    fn session_url(&self, c: &Connection, session: &str) -> String {
        format!("{}/sessions/{}", self.org(c), Self::part(session))
    }
    pub async fn connect(&self, token: &str, org: Option<&str>) -> Result<Connection> {
        validate(
            token.starts_with("cog_") && !token.contains(char::is_whitespace),
            "Use a Devin personal access token or service-user key beginning with cog_.",
        )?;
        let mut request = self
            .http
            .get(format!("{}/v3/self", self.api))
            .bearer_auth(token);
        if let Some(org) = org.filter(|v| !v.is_empty()) {
            request = request.header("x-cog-org-id", org);
        }
        let profile = self.json(request).await?;
        let principal_type = profile["principal_type"].as_str().unwrap_or("");
        let (principal_id, principal_name) = match principal_type {
            "pat_user" => (profile["user_id"].as_str(), profile["user_name"].as_str()),
            "service_user" => (
                profile["service_user_id"].as_str(),
                profile["service_user_name"].as_str(),
            ),
            _ => {
                return Err(Error::Invalid(
                    "Use a Devin personal access token or service-user key.".into(),
                ));
            }
        };
        let principal_id = principal_id.ok_or_else(|| {
            Error::Unavailable("Devin did not return an account identity.".into())
        })?;
        let org_id = org
            .filter(|s| !s.is_empty())
            .or_else(|| profile["devin_sessions_org_id"].as_str())
            .or_else(|| profile["org_id"].as_str())
            .ok_or_else(|| {
                Error::Invalid(
                    "Enter the organization ID shown at the top of Devin Settings → Devin API."
                        .into(),
                )
            })?;
        validate(
            org_id.starts_with("org-")
                && org_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "Enter a valid Devin organization ID.",
        )?;
        let connection = Connection {
            token: token.into(),
            org_id: org_id.into(),
            principal_id: principal_id.into(),
            principal_name: principal_name.unwrap_or(principal_id).into(),
            revision: id(),
        };
        // Verify organization access without creating or charging for a session.
        self.json(
            self.http
                .get(format!("{}/sessions", self.org(&connection)))
                .query(&[("first", "1")])
                .bearer_auth(token),
        )
        .await?;
        Ok(connection)
    }
    pub async fn start(&self, c: &Connection, task: &str, request: &str) -> Result<Session> {
        let prompt = format!(
            "The owner approved this Litecom request. Use the organization's existing context and permitted tools. Do not send the final reply or attachments to the requester through another channel. Return a complete reply for the owner to review in Litecom. Upload requested files as attachments to this session, and put their attachment IDs or exact filenames in files. Do not attach unrelated material or credentials. Explain permission denials and unfinished work honestly. Request (JSON): {}",
            serde_json::to_string(request)?
        );
        // No model, output-token, ACU, bypass-approval, or security-profile override.
        let value=self.json(self.http.post(format!("{}/sessions",self.org(c))).bearer_auth(&c.token).json(&json!({
            "prompt":prompt,"title":"Litecom request","tags":["litecom",task],
            "structured_output_schema":{"type":"object","properties":{"text":{"type":"string"},"files":{"type":"array","items":{"type":"string"}}},"required":["text","files"],"additionalProperties":false},
            "structured_output_required":true
        }))).await?;
        Ok(serde_json::from_value(value)?)
    }
    pub async fn get(&self, c: &Connection, session: &str) -> Result<Session> {
        Ok(serde_json::from_value(
            self.json(
                self.http
                    .get(self.session_url(c, session))
                    .bearer_auth(&c.token),
            )
            .await?,
        )?)
    }
    pub async fn terminate(&self, c: &Connection, session: &str) -> Result<()> {
        let response = self
            .http
            .delete(self.session_url(c, session))
            .bearer_auth(&c.token)
            .send()
            .await
            .map_err(|_| {
                Error::Unavailable(
                    "Could not confirm that Devin stopped. Check the session in Devin.".into(),
                )
            })?;
        if !response.status().is_success() {
            return Err(Error::Unavailable(
                "Devin has not confirmed termination. Check its session before retrying.".into(),
            ));
        }
        Ok(())
    }
    pub async fn attachments(&self, c: &Connection, session: &str) -> Result<Vec<Attachment>> {
        Ok(serde_json::from_value(
            self.json(
                self.http
                    .get(format!("{}/attachments", self.session_url(c, session)))
                    .bearer_auth(&c.token),
            )
            .await?,
        )?)
    }
    pub async fn download(&self, c: &Connection, file: &Attachment) -> Result<reqwest::Response> {
        validate(
            !file.attachment_id.is_empty(),
            "Devin returned an invalid attachment ID.",
        )?;
        let url = format!(
            "{}/attachments/{}/{}",
            self.org(c),
            Self::part(&file.attachment_id),
            Self::part(&file.name)
        );
        let mut response = self
            .http
            .get(url)
            .bearer_auth(&c.token)
            .send()
            .await
            .map_err(|_| {
                Error::Unavailable("The Devin attachment could not be downloaded.".into())
            })?;
        // The documented endpoint redirects to S3 or Devin's IP-enforcing proxy.
        // Signed redirect requests never carry the user's Devin bearer token.
        let mut visited = std::collections::HashSet::new();
        while response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    Error::Unavailable("Devin did not provide a download location.".into())
                })?;
            let url = url::Url::parse(location).map_err(|_| {
                Error::Unavailable("Devin returned an invalid download location.".into())
            })?;
            validate(
                visited.insert(url.clone()),
                "Devin returned a redirect loop.",
            )?;
            response = signed_download(url).await?;
        }
        if !response.status().is_success() {
            return Err(Error::Unavailable(
                "The Devin attachment is unavailable.".into(),
            ));
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downloads_reject_private_and_tunnel_addresses() {
        for value in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.168.1.1",
            "198.18.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2002:7f00:1::1",
            "64:ff9b::7f00:1",
        ] {
            assert!(!public_download_ip(value.parse().unwrap()), "{value}");
        }
        for value in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_download_ip(value.parse().unwrap()));
        }
        assert!(owner_session_url("https://app.devin.ai/sessions/one").is_some());
        for value in [
            "javascript:alert(1)",
            "https://app.devin.ai.evil.test/",
            "https://secret@app.devin.ai/",
            "https://app.devin.ai:8443/",
        ] {
            assert!(owner_session_url(value).is_none());
        }
    }
    #[tokio::test]
    async fn signed_redirects_reject_untrusted_origins_before_network_access() {
        for value in [
            "http://api.devin.ai/file",
            "https://example.test/file",
            "https://x.amazonaws.com.evil.test/file",
            "https://token@x.amazonaws.com/file",
            "https://api.devin.ai:8443/file",
        ] {
            assert!(signed_download(value.parse().unwrap()).await.is_err());
        }
    }
}
