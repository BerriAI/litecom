//! Provider-authenticated incoming replies. No browser session or agent token can substitute for a provider signature.
use crate::{
    api::AppState,
    error::{Error, Result, require},
    model::Decision,
};
use axum::{Json, body::Bytes, extract::State, http::HeaderMap, response::IntoResponse};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct ChannelAction {
    pub token: String,
    pub code: String,
    pub body: String,
    pub read_only: bool,
    pub actions: bool,
}
pub(crate) enum ChannelActor {
    Phone(String),
    Email(String),
    Slack {
        team: String,
        user: String,
        channel: String,
    },
}
impl ChannelActor {
    pub(crate) fn from_destination(kind: &str, value: &str) -> Result<Self> {
        match kind {
            "phone" => Ok(Self::Phone(value.into())),
            "email" => Ok(Self::Email(value.into())),
            "slack" => {
                let d: crate::slack::SlackDestination = serde_json::from_str(value)?;
                Ok(Self::Slack {
                    team: d.team,
                    user: d.user,
                    channel: d.channel,
                })
            }
            _ => Err(Error::NotFound),
        }
    }
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Phone(v) => format!("phone:{v}"),
            Self::Email(v) => format!("email:{v}"),
            Self::Slack {
                team,
                user,
                channel,
            } => format!("slack:{team}:{user}:{channel}"),
        }
    }
}
pub(crate) enum ReplyOutcome {
    Decided(Decision),
    AlreadyDecided(Decision),
}
impl ReplyOutcome {
    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::Decided(Decision::Approve) => "Approved. Your agent can continue.",
            Self::Decided(Decision::Decline) => "Declined. Permission was not granted.",
            Self::AlreadyDecided(Decision::Approve) => {
                "This was already approved. It will not run a second time."
            }
            Self::AlreadyDecided(Decision::Decline) => {
                "This was already declined. It will not run."
            }
        }
    }
}
pub(crate) fn decision(word: &str) -> Option<Decision> {
    if word.eq_ignore_ascii_case("approve") || read_only_word(word) {
        Some(Decision::Approve)
    } else if word.eq_ignore_ascii_case("decline") {
        Some(Decision::Decline)
    } else {
        None
    }
}
pub(crate) fn read_only_word(word: &str) -> bool {
    word.eq_ignore_ascii_case("readonly") || word.eq_ignore_ascii_case("read-only")
}
fn text_header<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Unauthorized)
}
pub(crate) fn verify_slack(headers: &HeaderMap, body: &[u8], secret: &str, now: i64) -> Result<()> {
    let timestamp = text_header(headers, "x-slack-request-timestamp")?;
    let time = timestamp.parse::<i64>().map_err(|_| Error::Unauthorized)?;
    require(now.abs_diff(time) <= 300, "This Slack request expired.")?;
    let signature = text_header(headers, "x-slack-signature")?
        .strip_prefix("v0=")
        .ok_or(Error::Unauthorized)?;
    let signature = hex::decode(signature).map_err(|_| Error::Unauthorized)?;
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| Error::Unauthorized)?;
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    mac.verify_slice(&signature)
        .map_err(|_| Error::Unauthorized)
}
fn form(body: &[u8]) -> Vec<(String, String)> {
    url::form_urlencoded::parse(body).into_owned().collect()
}
fn field<'a>(params: &'a [(String, String)], key: &str) -> Result<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or(Error::Unauthorized)
}
fn failure(error: Error) -> Result<String> {
    match error {
        Error::Internal(_) | Error::Unavailable(_)=>Err(error),
        _=>Ok("This reply could not approve the request. It may have changed, been cancelled, or belong to another account. Check the latest Litecom message.".into()),
    }
}
pub(crate) async fn agentphone(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let provider = s.providers.phone_provider().ok_or(Error::Unauthorized)?;
    let Some(incoming) = provider.incoming(&headers, &body, crate::store::now())? else {
        return Ok(Json(json!({"received":true})));
    };
    let receipt = crate::crypto::hash(&body);
    let ignore = {
        let db = s.db()?;
        db.phone_event_seen(&receipt)? || !db.phone_notifications_enabled(&incoming.from)?
    };
    if ignore {
        return Ok(Json(json!({"received":true})));
    }
    let command = incoming.message.trim();
    if ["stop", "stopall", "unsubscribe", "cancel", "end", "quit"]
        .iter()
        .any(|word| command.eq_ignore_ascii_case(word))
    {
        let db = s.db()?;
        db.transaction(|| {
            if !db.phone_event_seen(&receipt)? {
                db.stop_phone_notifications(&incoming.from)?;
                db.record_phone_event(&receipt)?;
            }
            Ok(())
        })?;
        return Ok(Json(json!({"received":true})));
    }
    let words = command.split_whitespace().collect::<Vec<_>>();
    let feedback = if let [word, code] = words.as_slice() {
        if let Some(decision) = decision(word) {
            let result = s.db()?.channel_decide_access(
                &ChannelActor::Phone(incoming.from.clone()),
                code,
                decision,
                read_only_word(word),
            );
            match result {
                // The committed outbox sends the update to every connected channel.
                // A replay returns the existing decision; it cannot claim another run.
                Ok(ReplyOutcome::Decided(_)) => None,
                Ok(outcome) => Some(outcome.message().to_owned()),
                Err(error) => Some(failure(error)?),
            }
        } else {
            Some(phone_help())
        }
    } else {
        Some(phone_help())
    };
    if let Some(message) = feedback {
        // AgentPhone's webhook response body does not send an SMS. Use its message API
        // for help/stale replies. A send failure returns 503 so their webhook retry can recover.
        s.providers.sms(&incoming.from, &message).await?;
    }
    s.db()?.record_phone_event(&receipt)?;
    Ok(Json(json!({"received":true})))
}
fn phone_help() -> String {
    "Reply APPROVE, READONLY or DECLINE followed by the code in your request.".into()
}
#[derive(Deserialize)]
struct SlackId {
    id: String,
}
#[derive(Deserialize)]
struct SlackButton {
    action_id: String,
    #[serde(default)]
    value: String,
}
#[derive(Deserialize)]
struct SlackPayload {
    #[serde(rename = "type")]
    kind: String,
    user: SlackId,
    team: SlackId,
    channel: SlackId,
    actions: Vec<SlackButton>,
}
pub(crate) async fn slack(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let secret = s
        .providers
        .slack_signing_secret()
        .ok_or(Error::Unauthorized)?;
    verify_slack(&headers, &body, secret, crate::store::now())?;
    let params = form(&body);
    let payload: SlackPayload =
        serde_json::from_str(field(&params, "payload")?).map_err(|_| Error::Unauthorized)?;
    require(
        payload.kind == "block_actions" && payload.actions.len() == 1,
        "Unsupported Slack action.",
    )?;
    let button = &payload.actions[0];
    if button.action_id == "litecom_view" || button.action_id.starts_with("litecom_view_") {
        return Ok(Json(json!({})));
    }
    let decision = match button.action_id.as_str() {
        "litecom_approve" | "litecom_read_only" => Decision::Approve,
        "litecom_decline" => Decision::Decline,
        _ => return Err(Error::Unauthorized),
    };
    let actor = ChannelActor::Slack {
        team: payload.team.id,
        user: payload.user.id,
        channel: payload.channel.id,
    };
    let message = match s.db()?.channel_decide_access(
        &actor,
        &button.value,
        decision,
        button.action_id == "litecom_read_only",
    ) {
        Ok(outcome) => outcome.message().into(),
        Err(error) => failure(error)?,
    };
    // Original message updates come from the durable outbox. Never call a supplied response_url.
    Ok(Json(
        json!({"response_type":"ephemeral","replace_original":false,"text":message}),
    ))
}
