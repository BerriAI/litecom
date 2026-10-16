//! Read access is the intersection of the owner's membership and the app's
//! access, checked on every request. Bot visibility alone is never authority.
use crate::{
    api::AppState,
    error::{Error, Result, require, validate},
    model::*,
    read_tools::{SlackRead, SlackResource},
    store::Store,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};

impl Store {
    pub(crate) fn readonly_slack_connection(
        &self,
        p: &Principal,
        task_id: &str,
        lease: &str,
        team: Option<&str>,
    ) -> Result<(String, crate::slack::SlackDestination)> {
        self.check_read_lease(p, task_id, lease)?;
        let rows=self.db.prepare("SELECT id,destination FROM channels WHERE user_id=? AND kind='slack' AND verified=1")?.query_map([&p.user_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut matches = Vec::new();
        for (id, data) in rows {
            let raw: String = self.vault.open(&data, &format!("channel:{id}"))?;
            let connection: crate::slack::SlackDestination = serde_json::from_str(&raw)?;
            if team.is_none_or(|t| t == connection.team) {
                matches.push((id, connection));
            }
        }
        validate(
            matches.len() == 1,
            "Choose a linked Slack workspace. Connect Slack in Litecom Settings if none is linked.",
        )?;
        Ok(matches.remove(0))
    }
}

#[derive(Deserialize)]
pub(crate) struct Input {
    lease: String,
    read: SlackRead,
}
pub(crate) async fn read(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(task): Path<String>,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    let p = s.principal(&h, true)?;
    let (id, old) =
        s.db()?
            .readonly_slack_connection(&p, &task, &input.lease, input.read.team.as_deref())?;
    let slack = s.providers.slack.as_ref().ok_or(Error::Unauthorized)?;
    let connection = slack.refresh(s.providers.http(), &old).await?;
    s.db()?.refresh_channel_destination(
        &id,
        &serde_json::to_string(&old)?,
        &serde_json::to_string(&connection)?,
    )?;
    let query = &input.read;
    let cursor = query.cursor.as_deref().unwrap_or("");
    validate(cursor.len() <= 4096, "This Slack cursor is invalid.")?;
    let response = if matches!(query.resource, SlackResource::Channels) {
        // users.conversations is scoped to the linked owner; private channels
        // additionally require the bot to be a member under Slack's API rules.
        let value = slack
            .read(
                s.providers.http(),
                &connection,
                "users.conversations",
                &[
                    ("user", connection.user.clone()),
                    ("types", "public_channel,private_channel".into()),
                    ("limit", "100".into()),
                    ("cursor", cursor.into()),
                ],
            )
            .await?;
        let channels = value["channels"]
            .as_array()
            .ok_or_else(|| Error::Unavailable("Slack did not return channels.".into()))?;
        json!({"channels":channels.iter().map(|c|json!({"id":c["id"],"name":c["name"]})).collect::<Vec<_>>(),"next_cursor":value["response_metadata"]["next_cursor"]})
    } else {
        let channel = query
            .channel
            .as_deref()
            .ok_or_else(|| Error::Invalid("Choose a Slack channel.".into()))?;
        validate(
            channel.len() <= 32
                && channel.starts_with(['C', 'G'])
                && channel.bytes().all(|c| c.is_ascii_alphanumeric()),
            "Use a Slack channel ID.",
        )?;
        let mut page = String::new();
        let mut cursors = std::collections::HashSet::new();
        loop {
            let members = slack
                .read(
                    s.providers.http(),
                    &connection,
                    "conversations.members",
                    &[
                        ("channel", channel.into()),
                        ("limit", "1000".into()),
                        ("cursor", page.clone()),
                    ],
                )
                .await?;
            let members_list = members["members"].as_array().ok_or(Error::Unauthorized)?;
            if members_list
                .iter()
                .any(|m| m.as_str() == Some(&connection.user))
            {
                break;
            }
            page = members["response_metadata"]["next_cursor"]
                .as_str()
                .unwrap_or("")
                .into();
            require(
                !page.is_empty() && cursors.insert(page.clone()),
                "The owner is not a member of this Slack channel.",
            )?;
            s.db()?.check_read_lease(&p, &task, &input.lease)?;
        }
        let mut args = vec![
            ("channel", channel.into()),
            ("limit", "100".into()),
            ("cursor", cursor.into()),
        ];
        let method = if matches!(query.resource, SlackResource::Thread) {
            let ts = query
                .thread_ts
                .as_deref()
                .ok_or_else(|| Error::Invalid("Provide the thread timestamp.".into()))?;
            validate(
                ts.len() <= 32
                    && ts.contains('.')
                    && ts.bytes().all(|b| b.is_ascii_digit() || b == b'.'),
                "Invalid thread timestamp.",
            )?;
            args.push(("ts", ts.into()));
            "conversations.replies"
        } else {
            "conversations.history"
        };
        s.db()?.check_read_lease(&p, &task, &input.lease)?;
        let value = slack
            .read(s.providers.http(), &connection, method, &args)
            .await?;
        let messages = value["messages"]
            .as_array()
            .ok_or_else(|| Error::Unavailable("Slack did not return messages.".into()))?;
        json!({"messages":messages.iter().map(|m|json!({"user":m["user"],"text":m["text"],"ts":m["ts"],"thread_ts":m["thread_ts"]})).collect::<Vec<_>>(),"next_cursor":value["response_metadata"]["next_cursor"]})
    };
    s.db()?.check_read_lease(&p, &task, &input.lease)?;
    Ok(Json(response))
}
