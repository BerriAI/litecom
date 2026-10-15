//! AgentPhone transports messages; all identity and approval decisions remain in Litecom.
use crate::error::{Error, Result};
use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone)]
pub(crate) struct AgentPhone {
    api: String,
    key: String,
    agent: String,
    number: String,
    webhook_secret: String,
}

impl AgentPhone {
    pub(crate) fn from_env(get: &impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        match (
            get("AGENTPHONE_API_KEY"),
            get("AGENTPHONE_AGENT_ID"),
            get("AGENTPHONE_NUMBER_ID"),
            get("AGENTPHONE_WEBHOOK_SECRET"),
        ) {
            (None, None, None, None) => Ok(None),
            (Some(key), Some(agent), Some(number), Some(webhook_secret)) => Ok(Some(Self {
                api: "https://api.agentphone.ai".into(),
                key, agent, number, webhook_secret,
            })),
            _ => Err(Error::Unavailable(
                "Set AGENTPHONE_API_KEY, AGENTPHONE_AGENT_ID, AGENTPHONE_NUMBER_ID, and AGENTPHONE_WEBHOOK_SECRET together. See docs/approval-channels.md.".into(),
            )),
        }
    }

    pub(crate) async fn send(
        &self,
        client: &reqwest::Client,
        to: &str,
        body: &str,
    ) -> Result<String> {
        let response = client
            .post(format!("{}/v1/messages", self.api))
            .bearer_auth(&self.key)
            .json(
                &json!({"agent_id":self.agent,"number_id":self.number,"to_number":to,"body":body}),
            )
            .send()
            .await
            .map_err(|_| {
                Error::Unavailable("AgentPhone could not be reached. Try again shortly.".into())
            })?;
        if !response.status().is_success() {
            return Err(Error::Unavailable(format!(
                "AgentPhone did not accept this message (HTTP {}). Check the sender's configuration and delivery status in AgentPhone.",
                response.status().as_u16()
            )));
        }
        #[derive(Deserialize)]
        struct Sent {
            id: String,
            status: String,
        }
        let sent: Sent = response.json().await.map_err(|_| {
            Error::Unavailable("AgentPhone returned an incomplete message response.".into())
        })?;
        if sent.id.is_empty()
            || matches!(
                sent.status.as_str(),
                "failed" | "undelivered" | "cancelled" | "canceled"
            )
        {
            return Err(Error::Unavailable(
                "AgentPhone could not send this message. Check its delivery status.".into(),
            ));
        }
        Ok(sent.id)
    }

    pub(crate) fn incoming(
        &self,
        headers: &HeaderMap,
        raw: &[u8],
        now: i64,
    ) -> Result<Option<Incoming>> {
        verify(headers, raw, &self.webhook_secret, now)?;
        let event: Event = serde_json::from_slice(raw)
            .map_err(|_| Error::Invalid("Invalid AgentPhone callback.".into()))?;
        // Reactions, calls and group messages never carry approval authority. The signed
        // envelope is authoritative; X-Webhook-Event is only an unsigned routing hint.
        if event.event != "agent.message"
            || !matches!(event.channel.as_str(), "sms" | "mms" | "imessage")
        {
            return Ok(None);
        }
        if event.agent_id != self.agent {
            return Err(Error::Unauthorized);
        }
        let data: Incoming = serde_json::from_value(event.data)
            .map_err(|_| Error::Invalid("Invalid AgentPhone message.".into()))?;
        if data.number_id != self.number {
            return Err(Error::Unauthorized);
        }
        if data.direction != "inbound" || data.group.is_some() || data.sender_identifier.is_some() {
            return Ok(None);
        }
        // A phone account must never be selected by an email address, group ID or display name.
        if !data.from.starts_with('+')
            || !data.from[1..].bytes().all(|v| v.is_ascii_digit())
            || data.from.len() < 2
        {
            return Err(Error::Unauthorized);
        }
        Ok(Some(data))
    }

    #[cfg(test)]
    pub(crate) fn test_api(api: &str) -> Self {
        Self {
            api: api.into(),
            key: "phone-key".into(),
            agent: "agt_test".into(),
            number: "num_test".into(),
            webhook_secret: "phone-secret".into(),
        }
    }
}

#[derive(Deserialize)]
struct Event {
    event: String,
    #[serde(default)]
    channel: String,
    #[serde(default, rename = "agentId")]
    agent_id: String,
    #[serde(default)]
    data: serde_json::Value,
}

#[derive(Deserialize)]
pub(crate) struct Incoming {
    #[serde(rename = "numberId")]
    number_id: String,
    pub from: String,
    #[serde(default)]
    pub message: String,
    direction: String,
    #[serde(default)]
    group: Option<serde_json::Value>,
    #[serde(default, rename = "senderIdentifier")]
    sender_identifier: Option<serde_json::Value>,
}

pub(crate) fn verify(headers: &HeaderMap, raw: &[u8], secret: &str, now: i64) -> Result<()> {
    let header = |key: &str| -> Result<&str> {
        let mut values = headers.get_all(key).iter();
        let value = values
            .next()
            .and_then(|v| v.to_str().ok())
            .ok_or(Error::Unauthorized)?;
        if values.next().is_some() {
            return Err(Error::Unauthorized);
        }
        Ok(value)
    };
    let timestamp = header("x-webhook-timestamp")?;
    let time: i64 = timestamp.parse().map_err(|_| Error::Unauthorized)?;
    if now.abs_diff(time) > 300 {
        return Err(Error::Unauthorized);
    }
    let signature = header("x-webhook-signature")?
        .strip_prefix("sha256=")
        .ok_or(Error::Unauthorized)?;
    let signature = hex::decode(signature).map_err(|_| Error::Unauthorized)?;
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| Error::Unauthorized)?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(raw);
    mac.verify_slice(&signature)
        .map_err(|_| Error::Unauthorized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn credentials_are_complete_or_disabled() {
        let keys = [
            "AGENTPHONE_API_KEY",
            "AGENTPHONE_AGENT_ID",
            "AGENTPHONE_NUMBER_ID",
            "AGENTPHONE_WEBHOOK_SECRET",
        ];
        for mask in 0..16 {
            let value = AgentPhone::from_env(&|name| {
                keys.iter()
                    .position(|key| *key == name)
                    .filter(|i| mask & (1 << i) != 0)
                    .map(|_| "fixture".into())
            });
            match mask {
                0 => assert!(value.unwrap().is_none()),
                15 => assert!(value.unwrap().is_some()),
                _ => assert!(value.is_err()),
            }
        }
    }

    #[tokio::test]
    async fn sends_complete_utf8_message_and_does_not_follow_redirects_or_hide_failures() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let saved = captured.clone();
        let redirects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let redirected = redirects.clone();
        let app = axum::Router::new()
            .route(
                "/v1/messages",
                axum::routing::post(
                    move |headers: HeaderMap, axum::Json(value): axum::Json<serde_json::Value>| {
                        let saved = saved.clone();
                        async move {
                            assert_eq!(headers["authorization"], "Bearer phone-key");
                            assert_eq!(value["agent_id"], "agt_test");
                            assert_eq!(value["number_id"], "num_test");
                            saved.lock().unwrap().push(value.clone());
                            let status = match value["to_number"].as_str().unwrap() {
                                "+10001" => 429,
                                "+10002" => 307,
                                "+10003" => 503,
                                _ => 200,
                            };
                            let response = match value["to_number"].as_str().unwrap() {
                                "+10004" => json!({"id":"msg_error","status":"failed"}),
                                "+10005" => json!({"status":"sent"}),
                                "+10006" => json!({"id":"","status":"sent"}),
                                _ => json!({"id":"msg_accepted","status":"queued"}),
                            };
                            (
                                axum::http::StatusCode::from_u16(status).unwrap(),
                                [("location", "/must-not-follow")],
                                axum::Json(response),
                            )
                        }
                    },
                ),
            )
            .route(
                "/must-not-follow",
                axum::routing::any(move || {
                    redirected.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = AgentPhone::test_api(&format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let text = "Read the complete request ☃\n".repeat(200);
        assert_eq!(
            provider.send(&client, "+14155550102", &text).await.unwrap(),
            "msg_accepted"
        );
        assert_eq!(captured.lock().unwrap()[0]["body"], text);
        for phone in ["+10001", "+10002", "+10003", "+10004", "+10005", "+10006"] {
            assert!(
                provider.send(&client, phone, "fixture").await.is_err(),
                "{phone}"
            );
        }
        assert_eq!(redirects.load(std::sync::atomic::Ordering::SeqCst), 0);
        server.abort();
    }

    #[test]
    fn verifies_raw_body_timestamp_and_constant_time_signature() {
        // Independently generated with Python's hmac/hashlib, following AgentPhone's documented algorithm.
        let raw = br#"{"event":"agent.message","message":"Approve \u2603"}"#;
        let mut headers = HeaderMap::new();
        headers.insert("x-webhook-timestamp", "1792087200".parse().unwrap());
        headers.insert(
            "x-webhook-signature",
            "sha256=cc378fb5c325d8800931693a71e329f2cf44218f12203d7bd6a34ba3ae3ca8e7"
                .parse()
                .unwrap(),
        );
        verify(&headers, raw, "whsec_fixture", 1792087200).unwrap();
        assert!(verify(&headers, raw, "other-secret", 1792087200).is_err());
        assert!(verify(&headers, raw, "whsec_fixture", 1792087501).is_err());
        assert!(verify(&headers, raw, "whsec_fixture", 1792086899).is_err());
        let mut changed = raw.to_vec();
        changed.push(b' ');
        assert!(verify(&headers, &changed, "whsec_fixture", 1792087200).is_err());
        headers.append("x-webhook-timestamp", "1792087200".parse().unwrap());
        assert!(verify(&headers, raw, "whsec_fixture", 1792087200).is_err());
    }
}
