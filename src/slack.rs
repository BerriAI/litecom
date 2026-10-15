use crate::{
    crypto::{hash, secret},
    error::{Error, Result, conflict, require},
    model::Principal,
    store::Store,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Slack {
    pub client_id: String,
    pub client_secret: String,
    api_base: String,
    pub(crate) agent_enabled: bool,
}
pub(crate) fn scopes(agent: bool) -> Vec<&'static str> {
    let mut scopes = vec!["chat:write", "im:write"];
    if agent {
        scopes.extend(["app_mentions:read", "channels:history", "groups:history"]);
    }
    scopes
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SlackDestination {
    pub token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub user: String,
    pub team: String,
    pub channel: String,
}
#[derive(Serialize, Deserialize)]
struct LinkData {
    destination: Option<SlackDestination>,
}
pub struct SlackStart {
    pub state: String,
}

impl Store {
    pub fn begin_slack(&self, p: &Principal) -> Result<SlackStart> {
        p.human()?;
        self.transaction(|| {
            let count:i64=self.db.query_row("SELECT COUNT(*) FROM oauth_links WHERE user_id=? AND expires_at>?",params![p.user_id,self.time()],|r|r.get(0))?;
            if count>=5{return Err(Error::RateLimited);}
            let state=secret();
            let key=hash(&state);
            self.db.execute("INSERT INTO oauth_links(state_hash,user_id,credential_id,data,expires_at) VALUES(?,?,?,?,?)",params![key,p.user_id,p.credential_id,self.vault.seal(&LinkData{destination:None},&format!("slack-link:{key}"))?,self.time()+600])?;
            Ok(SlackStart{state})
        })
    }
    pub fn take_slack_link(&self, state: &str) -> Result<()> {
        self.transaction(|| {
            let key = hash(state);
            let row = self
                .db
                .query_row(
                    "SELECT data,expires_at,phase FROM oauth_links WHERE state_hash=?",
                    [&key],
                    |r| {
                        Ok((
                            r.get::<_, Vec<u8>>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?
                .ok_or(Error::NotFound)?;
            conflict(
                row.1 > self.time() && row.2 == "pending",
                "This Slack connection expired or was already used. Start again from Settings.",
            )?;
            self.db.execute(
                "UPDATE oauth_links SET phase='exchanging' WHERE state_hash=?",
                [&key],
            )?;
            Ok(())
        })
    }
    pub fn stage_slack_link(&self, state: &str, destination: SlackDestination) -> Result<()> {
        let key = hash(state);
        let data = LinkData {
            destination: Some(destination),
        };
        let count=self.db.execute("UPDATE oauth_links SET data=?,phase='ready' WHERE state_hash=? AND phase='exchanging' AND expires_at>?",params![self.vault.seal(&data,&format!("slack-link:{key}"))?,key,self.time()])?;
        conflict(count == 1, "This Slack connection expired. Start it again.")
    }
    pub fn finish_slack_link(&self, p: &Principal, state: &str) -> Result<()> {
        p.human()?;
        self.transaction(||{
            let key=hash(state);
            let row=self.db.query_row("SELECT user_id,credential_id,data,expires_at,phase FROM oauth_links WHERE state_hash=?",[&key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?))).optional()?.ok_or(Error::NotFound)?;
            require(row.0==p.user_id && row.1==p.credential_id,"Finish connecting Slack in the browser that started it.")?;
            conflict(row.3>self.time(),"This Slack connection expired. Start again from Settings.")?;
            // A lost HTTP response must be safe to retry in the same browser.
            // Completion never reapplies an old destination or reenables a channel.
            if row.4 == "completed" { return Ok(()); }
            conflict(row.4=="ready","Slack has not finished connecting.")?;
            let data:LinkData=self.vault.open(&row.2,&format!("slack-link:{key}"))?;
            let destination=data.destination.ok_or_else(||Error::Conflict("Slack has not finished connecting.".into()))?;
            self.verified_channel(p,"slack",&serde_json::to_string(&destination)?)?;
            self.db.execute("UPDATE oauth_links SET phase='completed',data=? WHERE state_hash=?",params![self.vault.seal(&LinkData{destination:None},&format!("slack-link:{key}"))?,key])?;
            Ok(())
        })
    }
    pub fn refresh_channel_destination(&self, id: &str, old: &str, new: &str) -> Result<()> {
        self.transaction(|| {
            let row: Option<Vec<u8>> = self
                .db
                .query_row(
                    "SELECT destination FROM channels WHERE id=? AND verified=1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(bytes) = row {
                let existing: String = self.vault.open(&bytes, &format!("channel:{id}"))?;
                if existing == old {
                    self.db.execute(
                        "UPDATE channels SET destination=? WHERE id=?",
                        params![self.vault.seal(&new, &format!("channel:{id}"))?, id],
                    )?;
                }
            }
            Ok(())
        })
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn field(value: &Value, name: &str) -> Result<String> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::Unavailable("Slack returned an incomplete connection. Please reconnect.".into())
        })
}
async fn response(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request
        .send()
        .await
        .map_err(|_| Error::Unavailable("Slack could not be reached. Please retry.".into()))?;
    if !response.status().is_success() {
        return Err(Error::Unavailable(
            "Slack did not accept this request. Retry shortly or check the app connection.".into(),
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| Error::Unavailable("Slack returned an invalid response.".into()))?;
    if value["ok"] != true {
        return Err(Error::Unavailable(
            "Slack did not complete the request. Reconnect the Slack app if this continues.".into(),
        ));
    }
    Ok(value)
}
impl Slack {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            api_base: "https://slack.com/api".into(),
            agent_enabled: false,
        }
    }
    #[cfg(test)]
    pub(crate) fn test_api(client_id: String, client_secret: String, api_base: String) -> Self {
        Self {
            client_id,
            client_secret,
            api_base,
            agent_enabled: false,
        }
    }
    pub fn authorize_url(&self, origin: &str, start: &SlackStart) -> String {
        let mut url =
            url::Url::parse("https://slack.com/oauth/v2/authorize").expect("constant URL");
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.client_id.as_str()),
            ("scope", &scopes(self.agent_enabled).join(",")),
            ("redirect_uri", &format!("{origin}/oauth/slack/callback")),
            ("state", start.state.as_str()),
        ]);
        url.into()
    }
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        origin: &str,
        code: &str,
    ) -> Result<SlackDestination> {
        let value = response(
            http.post(format!("{}/oauth.v2.access", self.api_base))
                .basic_auth(&self.client_id, Some(&self.client_secret))
                .form(&[
                    ("code", code),
                    ("redirect_uri", &format!("{origin}/oauth/slack/callback")),
                ]),
        )
        .await?;
        if value["token_type"] != "bot" {
            return Err(Error::Unavailable(
                "Install the Litecom Slack app with its bot permissions.".into(),
            ));
        }
        let token = field(&value, "access_token")?;
        let user = field(&value["authed_user"], "id")?;
        let team = field(&value["team"], "id")?;
        let opened = response(
            http.post(format!("{}/conversations.open", self.api_base))
                .bearer_auth(&token)
                .json(&json!({"users":user})),
        )
        .await?;
        Ok(SlackDestination {
            token,
            refresh_token: value["refresh_token"].as_str().map(str::to_owned),
            expires_at: value["expires_in"].as_i64().map(|s| now() + s),
            user,
            team,
            channel: field(&opened["channel"], "id")?,
        })
    }
    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        destination: &SlackDestination,
    ) -> Result<SlackDestination> {
        if !destination.expires_at.is_some_and(|t| t <= now() + 300) {
            return Ok(destination.clone());
        }
        let refresh = destination
            .refresh_token
            .as_deref()
            .ok_or_else(|| Error::Unavailable("Reconnect Slack: its token expired.".into()))?;
        let value = response(
            http.post(format!("{}/oauth.v2.access", self.api_base))
                .basic_auth(&self.client_id, Some(&self.client_secret))
                .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh)]),
        )
        .await?;
        let mut updated = destination.clone();
        updated.token = field(&value, "access_token")?;
        updated.refresh_token = Some(field(&value, "refresh_token")?);
        updated.expires_at = value["expires_in"].as_i64().map(|s| now() + s);
        Ok(updated)
    }
    pub async fn notify(
        &self,
        http: &reqwest::Client,
        destination: &SlackDestination,
        body: &str,
        previous: Option<&str>,
        action: Option<&str>,
    ) -> Result<String> {
        let previous: Option<Value> = previous.and_then(|s| serde_json::from_str(s).ok());
        let ts = previous
            .as_ref()
            .filter(|v| v["channel"] == destination.channel)
            .and_then(|v| v["ts"].as_str());
        let method = if ts.is_some() {
            "chat.update"
        } else {
            "chat.postMessage"
        };
        let mut blocks = Vec::new();
        let mut chunk = String::new();
        let mut units = 0;
        for ch in body.chars() {
            if units + ch.len_utf16() > 3000 {
                blocks.push(json!({"type":"section","text":{"type":"plain_text","text":chunk}}));
                chunk = String::new();
                units = 0;
            }
            chunk.push(ch);
            units += ch.len_utf16();
        }
        if !chunk.is_empty() {
            blocks.push(json!({"type":"section","text":{"type":"plain_text","text":chunk}}));
        }
        if let Some(token) = action {
            blocks.push(json!({"type":"actions","elements":[
                {"type":"button","text":{"type":"plain_text","text":"Approve"},"style":"primary","action_id":"litecom_approve","value":token},
                {"type":"button","text":{"type":"plain_text","text":"Decline"},"action_id":"litecom_decline","value":token}
            ]}));
        }
        let mut payload = json!({"channel":destination.channel,"text":body,"blocks":blocks,"parse":"none","link_names":false,"unfurl_links":false,"unfurl_media":false});
        if let Some(ts) = ts {
            payload["ts"] = json!(ts);
        }
        let result = response(
            http.post(format!("{}/{method}", self.api_base))
                .bearer_auth(&destination.token)
                .json(&payload),
        )
        .await?;
        Ok(serde_json::to_string(
            &json!({"channel":destination.channel,"ts":field(&result,"ts")?}),
        )?)
    }

    pub(crate) async fn read(
        &self,
        http: &reqwest::Client,
        destination: &SlackDestination,
        method: &str,
        query: &[(&str, String)],
    ) -> Result<Value> {
        response(
            http.get(format!("{}/{method}", self.api_base))
                .bearer_auth(&destination.token)
                .query(query),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reply(
        &self,
        http: &reqwest::Client,
        destination: &SlackDestination,
        channel: &str,
        thread: &str,
        body: &str,
        previous: Option<&str>,
        message_id: &str,
    ) -> Result<String> {
        let mut payload = json!({"channel":channel,"text":body,"mrkdwn":false,"parse":"none","link_names":false,"unfurl_links":false,"unfurl_media":false});
        let method = if let Some(ts) = previous {
            payload["ts"] = json!(ts);
            "chat.update"
        } else {
            payload["thread_ts"] = json!(thread);
            payload["client_msg_id"] = json!(message_id);
            "chat.postMessage"
        };
        let value = response(
            http.post(format!("{}/{method}", self.api_base))
                .bearer_auth(&destination.token)
                .json(&payload),
        )
        .await?;
        field(&value, "ts")
    }
}
