//! Provider-authenticated incoming replies. No browser session or agent token can substitute for a provider signature.
use crate::{
    api::AppState,
    error::{Error, Result, require},
    model::Decision,
};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, header},
    response::IntoResponse,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct ChannelAction {
    pub token: String,
    pub code: String,
    pub body: String,
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
    if word.eq_ignore_ascii_case("approve") {
        Some(Decision::Approve)
    } else if word.eq_ignore_ascii_case("decline") {
        Some(Decision::Decline)
    } else {
        None
    }
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
pub(crate) fn verify_twilio(
    headers: &HeaderMap,
    params: &[(String, String)],
    url: &str,
    secret: &str,
) -> Result<()> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(text_header(headers, "x-twilio-signature")?)
        .map_err(|_| Error::Unauthorized)?;
    let mut params = params.to_vec();
    params.sort();
    params.dedup();
    // Twilio signs all parameters, including newly added ones. Reject ambiguous repeated names.
    require(
        !params.windows(2).any(|p| p[0].0 == p[1].0),
        "Ambiguous phone callback.",
    )?;
    let mut mac =
        Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).map_err(|_| Error::Unauthorized)?;
    mac.update(url.as_bytes());
    for (key, value) in params {
        mac.update(key.as_bytes());
        mac.update(value.as_bytes());
    }
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
pub(crate) async fn twilio(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let (sid, secret) = s
        .providers
        .twilio_verification()
        .ok_or(Error::Unauthorized)?;
    let params = form(&body);
    verify_twilio(
        &headers,
        &params,
        &format!("{}/webhooks/twilio", s.origin),
        secret,
    )?;
    require(
        field(&params, "AccountSid")? == sid,
        "Wrong Twilio account.",
    )?;
    let sender = field(&params, "From")?;
    let command = field(&params, "Body")?.trim();
    let words = command.split_whitespace().collect::<Vec<_>>();
    if command.eq_ignore_ascii_case("stop")
        || params.iter().any(|(k, v)| k == "OptOutType" && v == "STOP")
    {
        s.db()?.stop_phone_notifications(sender)?;
        return Ok((
            [(header::CONTENT_TYPE, "application/xml")],
            "<Response/>".to_owned(),
        ));
    }
    let message = if let [word, code] = words.as_slice() {
        if let Some(decision) = decision(word) {
            match s
                .db()?
                .channel_decide(&ChannelActor::Phone(sender.into()), code, decision)
            {
                Ok(outcome) => outcome.message().into(),
                Err(error) => failure(error)?,
            }
        } else {
            "Reply APPROVE followed by the code in your request, or DECLINE followed by that code."
                .into()
        }
    } else {
        "Reply APPROVE followed by the code in your request, or DECLINE followed by that code."
            .into()
    };
    Ok((
        [(header::CONTENT_TYPE, "application/xml")],
        format!("<Response><Message>{}</Message></Response>", xml(&message)),
    ))
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
#[derive(Deserialize)]
struct SlackId {
    id: String,
}
#[derive(Deserialize)]
struct SlackButton {
    action_id: String,
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
    let decision = match button.action_id.as_str() {
        "litecom_approve" => Decision::Approve,
        "litecom_decline" => Decision::Decline,
        _ => return Err(Error::Unauthorized),
    };
    let actor = ChannelActor::Slack {
        team: payload.team.id,
        user: payload.user.id,
        channel: payload.channel.id,
    };
    let message = match s.db()?.channel_decide(&actor, &button.value, decision) {
        Ok(outcome) => outcome.message().into(),
        Err(error) => failure(error)?,
    };
    // Original message updates come from the durable outbox. Never call a supplied response_url.
    Ok(Json(
        json!({"response_type":"ephemeral","replace_original":false,"text":message}),
    ))
}
