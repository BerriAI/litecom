use crate::{
    api::AppState,
    crypto::id,
    error::{Error, Result},
    model::Principal,
    store::Store,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Serialize)]
pub struct Channel {
    pub id: String,
    pub kind: String,
    pub verified: bool,
    pub delivery_status: Option<String>,
}
pub struct Delivery {
    pub event_id: i64,
    pub channel_id: String,
    pub destination: String,
    pub task_id: String,
    pub kind: String,
    pub attempts: i64,
    pub channel_kind: String,
    pub previous: Option<String>,
}
impl Store {
    pub(crate) fn enqueue_notifications(&self, event_id: i64) -> Result<()> {
        // Persist each delivery in the same transaction as its task transition.
        // A stopped delivery worker must not lose events, however long it is offline.
        self.db.execute("INSERT OR IGNORE INTO deliveries(event_id,channel_id,next_attempt) SELECT e.sequence,c.id,CASE WHEN e.kind='queued' THEN e.created_at+5 ELSE 0 END FROM events e JOIN tasks t ON t.id=e.task_id JOIN channels c ON c.user_id=t.owner_id OR c.user_id=t.requester_id WHERE e.sequence=? AND c.verified=1 AND e.kind IN ('queued','awaiting_approval','approved','declined','execution_finished','cancelled','received','download_verified','recovered','cloud_attention','permission_requested') AND (e.kind NOT IN ('queued','awaiting_approval','cloud_attention','permission_requested') OR c.user_id=t.owner_id)",[event_id])?;
        Ok(())
    }
    pub fn enable_phone(&self, p: &Principal) -> Result<()> {
        p.human()?;
        let phone = self.phone(&p.user_id)?;
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM channels WHERE user_id=? AND kind='phone'",
                [&p.user_id],
                |r| r.get(0),
            )
            .optional()?;
        let channel_id = existing.unwrap_or_else(id);
        self.db.execute("INSERT INTO channels(id,user_id,kind,destination,verified) VALUES(?,?,'phone',?,1) ON CONFLICT(user_id,kind) DO UPDATE SET destination=excluded.destination,verified=1",params![channel_id,p.user_id,self.vault.seal(&phone,&format!("channel:{channel_id}"))?])?;
        Ok(())
    }
    pub fn channels(&self, p: &Principal) -> Result<Vec<Channel>> {
        Ok(self
            .db
            .prepare("SELECT c.id,c.kind,c.verified,(SELECT d.state FROM deliveries d WHERE d.channel_id=c.id ORDER BY d.event_id DESC LIMIT 1) FROM channels c WHERE c.user_id=? ORDER BY c.kind")?
            .query_map([&p.user_id], |r| {
                Ok(Channel {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    verified: r.get(2)?,
                    delivery_status: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn disable_channel(&self, p: &Principal, channel_id: &str) -> Result<()> {
        p.human()?;
        self.db.execute("UPDATE deliveries SET state='cancelled' WHERE channel_id=? AND state IN ('pending','sending') AND EXISTS(SELECT 1 FROM channels WHERE id=? AND user_id=?)",params![channel_id,channel_id,p.user_id])?;
        self.db.execute(
            "UPDATE channels SET verified=0 WHERE id=? AND user_id=?",
            params![channel_id, p.user_id],
        )?;
        Ok(())
    }
    pub fn next_delivery(&self) -> Result<Option<Delivery>> {
        self.transaction(|| {
            self.db.execute("UPDATE deliveries SET state='superseded' WHERE state='pending' AND EXISTS(SELECT 1 FROM deliveries newer JOIN events ne ON ne.sequence=newer.event_id JOIN events old ON old.sequence=deliveries.event_id WHERE newer.channel_id=deliveries.channel_id AND newer.event_id>deliveries.event_id AND ne.task_id=old.task_id)",[])?;
            let row=self.db.query_row("SELECT d.event_id,d.channel_id,c.destination,e.task_id,e.kind,d.attempts,c.kind FROM deliveries d JOIN channels c ON c.id=d.channel_id JOIN events e ON e.sequence=d.event_id WHERE c.verified=1 AND c.kind IN ('phone','email','slack') AND d.state IN ('pending','sending') AND d.next_attempt<=? AND d.attempts<8 ORDER BY e.sequence,c.id LIMIT 1",[self.time()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?,r.get::<_,String>(6)?))).optional()?;
            let Some((event_id,channel_id,data,task_id,kind,attempts,channel_kind))=row else{return Ok(None)};
            self.db.execute("UPDATE deliveries SET state='sending',attempts=attempts+1,next_attempt=? WHERE event_id=? AND channel_id=?",params![self.time()+60,event_id,channel_id])?;
            let previous:Option<String>=self.db.query_row("SELECT d.remote_id FROM deliveries d JOIN events e ON e.sequence=d.event_id WHERE d.channel_id=? AND e.task_id=? AND d.state='accepted_by_provider' AND d.remote_id IS NOT NULL ORDER BY d.event_id DESC LIMIT 1",params![channel_id,task_id],|r|r.get(0)).optional()?;
            Ok(Some(Delivery{event_id,channel_id:channel_id.clone(),destination:self.vault.open(&data,&format!("channel:{channel_id}"))?,task_id,kind,attempts:attempts+1,channel_kind,previous}))
        })
    }
    pub fn delivery_result(
        &self,
        d: &Delivery,
        result: &std::result::Result<String, Error>,
    ) -> Result<()> {
        match result {
            Ok(remote_id) => {
                self.db.execute("UPDATE deliveries SET state='accepted_by_provider',remote_id=?,error=NULL WHERE event_id=? AND channel_id=? AND state='sending' AND attempts=?",params![remote_id,d.event_id,d.channel_id,d.attempts])?;
            }
            Err(_) => {
                self.db.execute("UPDATE deliveries SET state=?,error='Provider did not accept the notification.',next_attempt=? WHERE event_id=? AND channel_id=? AND state='sending' AND attempts=?",params![if d.attempts>=8{"failed"}else{"pending"},self.time()+(2_i64.pow(d.attempts as u32)*30).min(3600),d.event_id,d.channel_id,d.attempts])?;
            }
        }
        Ok(())
    }
}
pub async fn run(state: AppState) {
    loop {
        let next = state.db().and_then(|db| {
            db.recover()?;
            db.next_delivery()
        });
        match next {
            Ok(Some(d)) => {
                let title = match state.db().and_then(|db| db.delivery_title(&d)) {
                    Ok(title) => title,
                    Err(error) => {
                        tracing::warn!(%error,"could not read notification state");
                        continue;
                    }
                };
                let body = format!("Litecom: {title} {}/request/{}", state.origin, d.task_id);
                let result = match d.channel_kind.as_str() {
                    "slack" => send_slack(&state, &d, &body).await,
                    "email" => {
                        state
                            .providers
                            .email(&d.destination, "Litecom request update", &body)
                            .await
                    }
                    _ => state.providers.sms(&d.destination, &body).await,
                };
                if let Err(error) = state.db().and_then(|db| db.delivery_result(&d, &result)) {
                    tracing::error!(?error, "could not record notification delivery");
                }
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_secs(3)).await,
            Err(error) => {
                tracing::error!(?error, "notification worker error");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
    }
}

async fn send_slack(state: &AppState, d: &Delivery, body: &str) -> Result<String> {
    if state.providers.dev {
        eprintln!(
            "[LOCAL DEVELOPMENT — no Slack message sent] {}: {body}",
            d.channel_id
        );
        return Ok("local-development".into());
    }
    let slack = state
        .providers
        .slack
        .as_ref()
        .ok_or_else(|| Error::Unavailable("Slack is not enabled on this server.".into()))?;
    let destination: crate::slack::SlackDestination = serde_json::from_str(&d.destination)?;
    let updated = slack.refresh(state.providers.http(), &destination).await?;
    state.db()?.refresh_channel_destination(
        &d.channel_id,
        &d.destination,
        &serde_json::to_string(&updated)?,
    )?;
    slack
        .notify(
            state.providers.http(),
            &updated,
            body,
            d.previous.as_deref(),
        )
        .await
}
