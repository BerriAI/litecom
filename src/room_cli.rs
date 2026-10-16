use crate::{client::Client, config::ClientConfig, rooms::*};
use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};

#[derive(Subcommand)]
pub enum ChannelCommand {
    /// Open a shared channel and join it from this chat. Does not start an agent.
    Create {
        members: Vec<String>,
        #[arg(long, default_value = "")]
        topic: String,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        session_key: Option<String>,
    },
    /// Find open channels you can join, optionally with a person.
    List {
        person: Option<String>,
        #[arg(long)]
        closed: bool,
    },
    /// Join by channel ID or by person (only if exactly one channel matches).
    Join {
        channel: String,
        #[arg(long)]
        session_key: Option<String>,
        #[arg(long, default_value = "")]
        label: String,
    },
    /// Read messages without joining. Use next_cursor as --after for the next page.
    Read {
        channel: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    /// Send, read, or wait in this chat. Reuse message ID and cursor on retries.
    Sync {
        channel: String,
        #[arg(long)]
        participant: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
        #[arg(long, requires = "message_id")]
        message: Option<String>,
        #[arg(long, requires = "message")]
        message_id: Option<String>,
        #[arg(long, default_value_t=0, value_parser=clap::value_parser!(u8).range(0..=25))]
        wait: u8,
    },
    /// Leave from this chat. Other joined chats keep their access.
    Leave {
        channel: String,
        #[arg(long)]
        participant: String,
    },
    /// Close a channel you opened; history remains available to its members.
    Close { channel: String },
}

/// Existing chats may not have the new MCP tools yet. Reuse the only saved
/// connection when no default exists; never guess between multiple accounts.
pub fn config(connection: &str) -> Result<ClientConfig> {
    if connection != "default" || ClientConfig::path(connection)?.exists() {
        return ClientConfig::load_named(connection);
    }
    let dir = crate::config::home()?.join("connections");
    let mut names = Vec::new();
    if dir.exists() {
        for item in std::fs::read_dir(dir)? {
            let path = item?.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Some(name) = path.file_stem().and_then(|n| n.to_str())
            {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    if names.len() == 1 {
        return ClientConfig::load_named(&names[0]);
    }
    if names.is_empty() {
        bail!("No saved Litecom connection. Connect your agent from your workspace first.");
    }
    bail!(
        "Choose your saved connection with --connection NAME: {}",
        names.join(", ")
    )
}
pub fn segment(id: &str) -> Result<&str> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| anyhow::anyhow!("Use the channel or participant ID returned by Litecom."))?;
    Ok(id)
}
pub fn query(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
pub async fn channel_id(client: &Client, target: &str) -> Result<String> {
    if uuid::Uuid::parse_str(target).is_ok() {
        return Ok(target.into());
    }
    let channels: Vec<Channel> = client
        .get(&format!("/api/rooms?person={}", query(target)))
        .await?;
    match channels.as_slice() {
        [channel] => Ok(channel.id.clone()),
        [] => bail!("No open channel with {target}. Open one with `litecom channel create` first."),
        _ => bail!(
            "Several channels match. Join by ID:\n{}",
            channels
                .iter()
                .map(|c| format!("{}  {}", c.id, c.topic))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}
pub async fn run(client: Client, action: ChannelCommand) -> Result<Value> {
    Ok(match action {
        ChannelCommand::Create {
            members,
            topic,
            request_id,
            session_key,
        } => {
            let input = CreateChannel {
                members,
                topic,
                request_id: request_id.unwrap_or_else(crate::crypto::id),
            };
            let channel: Channel = client
                .post("/api/rooms", serde_json::to_value(input)?)
                .await?;
            let key = session_key.unwrap_or_else(crate::crypto::id);
            let joined: Value = client
                .post(
                    &format!("/api/rooms/{}/join", channel.id),
                    json!({"session_key":key}),
                )
                .await?;
            json!({"channel":channel,"url":format!("{}/channel/{}",client.config.relay.trim_end_matches('/'),channel.id),"session_key":key,"joined":joined})
        }
        ChannelCommand::List { person, closed } => {
            client
                .get(&format!(
                    "/api/rooms?closed={closed}{}",
                    person
                        .map(|p| format!("&person={}", query(&p)))
                        .unwrap_or_default()
                ))
                .await?
        }
        ChannelCommand::Join {
            channel,
            session_key,
            label,
        } => {
            let id = channel_id(&client, &channel).await?;
            let key = session_key.unwrap_or_else(crate::crypto::id);
            let joined: Value = client
                .post(
                    &format!("/api/rooms/{id}/join"),
                    json!({"session_key":key,"label":label}),
                )
                .await?;
            json!({"session_key":key,"joined":joined})
        }
        ChannelCommand::Read { channel, after } => {
            client
                .get(&format!("/api/rooms/{}?after={after}", segment(&channel)?))
                .await?
        }
        ChannelCommand::Sync {
            channel,
            participant,
            after,
            message,
            message_id,
            wait,
        } => {
            client
                .post(
                    &format!("/api/rooms/{}/sync", segment(&channel)?),
                    serde_json::to_value(ChannelSync {
                        participant_id: participant,
                        after,
                        message,
                        message_id,
                        wait_seconds: wait,
                    })?,
                )
                .await?
        }
        ChannelCommand::Leave {
            channel,
            participant,
        } => {
            client
                .post(
                    &format!(
                        "/api/rooms/{}/leave/{}",
                        segment(&channel)?,
                        segment(&participant)?
                    ),
                    json!({}),
                )
                .await?
        }
        ChannelCommand::Close { channel } => {
            client
                .post(
                    &format!("/api/rooms/{}/close", segment(&channel)?),
                    json!({}),
                )
                .await?
        }
    })
}
