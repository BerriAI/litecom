//! SMTP replies are read from an operator-owned, dedicated IMAP mailbox over TLS.
//! Users only connect their email address; they never supply mailbox credentials.
use crate::{
    api::AppState,
    channel_replies::{ChannelActor, decision},
    crypto::hash,
    error::{Error, Result},
};
use futures_util::TryStreamExt;
use rusqlite::{OptionalExtension, params};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub(crate) struct Inbox {
    host: String,
    port: u16,
    user: String,
    password: String,
}
impl Inbox {
    pub(crate) fn from_env() -> Result<Option<Self>> {
        let get = |key| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        match (get("IMAP_HOST"), get("IMAP_USERNAME"), get("IMAP_PASSWORD")) {
            (None, None, None) => Ok(None),
            (Some(host), Some(user), Some(password)) => {
                if get("SMTP_HOST").is_none() || get("SMTP_FROM").is_none() {
                    return Err(Error::Invalid(
                        "Email replies require SMTP_HOST and SMTP_FROM for notifications.".into(),
                    ));
                }
                let port = get("IMAP_PORT")
                    .unwrap_or_else(|| "993".into())
                    .parse()
                    .map_err(|_| Error::Invalid("IMAP_PORT must be a port number.".into()))?;
                Ok(Some(Self {
                    host,
                    port,
                    user,
                    password,
                }))
            }
            _ => Err(Error::Invalid(
                "Set IMAP_HOST, IMAP_USERNAME, and IMAP_PASSWORD together for email approvals."
                    .into(),
            )),
        }
    }
}
struct Reply {
    sender: String,
    token: String,
    decision: crate::model::Decision,
    read_only: bool,
}
fn parse_reply(raw: &[u8]) -> Option<Reply> {
    let mail = mail_parser::MessageParser::default().parse(raw)?;
    // Vacation notices and delivery reports must never authorize work.
    if mail
        .header("Auto-Submitted")
        .and_then(|h| h.as_text())
        .is_some_and(|v| !v.eq_ignore_ascii_case("no"))
    {
        return None;
    }
    if mail.header("List-Id").is_some() {
        return None;
    }
    let from = mail.from()?;
    if from.iter().count() != 1 {
        return None;
    }
    let sender = from.first()?.address.as_deref()?.to_owned();
    let refs = mail.in_reply_to().as_text_list()?;
    if refs.len() != 1 {
        return None;
    }
    let (local, _domain) = refs[0].split_once('@')?;
    let token = local.strip_prefix("litecom-approve-")?;
    if token.len() != 43
        || !token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return None;
    }
    let body = mail.body_text(0)?;
    let first = body.lines().map(str::trim).find(|v| !v.is_empty())?;
    Some(Reply {
        sender,
        token: token.into(),
        decision: decision(first)?,
        read_only: crate::channel_replies::read_only_word(first),
    })
}

pub(crate) fn process(state: &AppState, raw: &[u8]) -> Result<bool> {
    let Some(reply) = parse_reply(raw) else {
        return Ok(false);
    };
    match state.db()?.channel_decide_access(
        &ChannelActor::Email(reply.sender),
        &reply.token,
        reply.decision,
        reply.read_only,
    ) {
        Ok(_) => Ok(true),
        Err(Error::Internal(e)) => Err(Error::Internal(e)),
        Err(Error::Unavailable(e)) => Err(Error::Unavailable(e)),
        Err(_) => Ok(false),
    }
}

async fn poll(state: &AppState, inbox: &Inbox) -> Result<()> {
    let root_store = tokio_rustls::rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    );
    let tls = tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let domain = tokio_rustls::rustls::pki_types::ServerName::try_from(inbox.host.clone())
        .map_err(|_| Error::Invalid("IMAP_HOST must be a TLS hostname.".into()))?;
    let tcp = tokio::net::TcpStream::connect((inbox.host.as_str(), inbox.port)).await?;
    let stream = tokio_rustls::TlsConnector::from(Arc::new(tls))
        .connect(domain, tcp)
        .await?;
    poll_stream(state, inbox, stream).await
}
async fn poll_stream<T>(state: &AppState, inbox: &Inbox, stream: T) -> Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mut client = async_imap::Client::new(stream);
    let greeting = client.read_response().await?;
    if greeting.is_none() {
        return Err(Error::Unavailable(
            "The IMAP server closed the connection.".into(),
        ));
    }
    let mut session = client
        .login(&inbox.user, &inbox.password)
        .await
        .map_err(|_| {
            Error::Unavailable("IMAP sign-in failed. Check the relay mailbox credentials.".into())
        })?;
    let mailbox = session.select("INBOX").await.map_err(imap_error)?;
    let validity = mailbox
        .uid_validity
        .ok_or_else(|| Error::Unavailable("IMAP did not provide UIDVALIDITY.".into()))?;
    let checkpoint = format!(
        "imap:{}:{validity}",
        hash(format!(
            "{}:{}:{}:INBOX",
            inbox.host, inbox.port, inbox.user
        ))
    );
    let mut after: u32 = state
        .db()?
        .db
        .query_row(
            "SELECT CAST(value AS TEXT) FROM settings WHERE key=?",
            [&checkpoint],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if let Some(next) = after.checked_add(1) {
        let mut ids = session
            .uid_search(format!("UID {next}:*"))
            .await
            .map_err(imap_error)?
            .into_iter()
            .filter(|id| *id > after)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        for id in ids {
            // PEEK preserves read/unread state. One message per fetch bounds memory by message size.
            let mut fetched = session
                .uid_fetch(id.to_string(), "(UID BODY.PEEK[])")
                .await
                .map_err(imap_error)?;
            while let Some(item) = fetched.try_next().await.map_err(imap_error)? {
                if item.uid == Some(id)
                    && let Some(body) = item.body()
                {
                    process(state, body)?;
                }
            }
            after = id;
            state.db()?.db.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![checkpoint,after.to_string()])?;
        }
    }
    session.logout().await.map_err(imap_error)?;
    Ok(())
}
fn imap_error(_: async_imap::error::Error) -> Error {
    Error::Unavailable(
        "The approval mailbox could not be read. Check the relay's IMAP connection.".into(),
    )
}
pub async fn run(state: AppState) {
    if state.providers.dev {
        return;
    }
    let Some(inbox) = state.providers.imap.clone() else {
        return;
    };
    loop {
        match tokio::time::timeout(Duration::from_secs(60), poll(&state, &inbox)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error,"email approval receiver"),
            Err(_) => tracing::warn!("email approval mailbox timed out; retrying"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_replies_to_a_capability_are_commands() {
        let token = crate::crypto::secret();
        let raw = format!(
            "From: Moe <moe@example.com>\r\nIn-Reply-To: <litecom-approve-{token}@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nApprove\r\n\r\nOn Tuesday, Litecom wrote:\r\n> Decline"
        );
        let reply = parse_reply(raw.as_bytes()).unwrap();
        assert_eq!(reply.sender, "moe@example.com");
        assert_eq!(reply.token, token);
        for bad in [
            raw.replace("\r\n\r\nApprove", "\r\n\r\n> Approve"),
            raw.replace("\r\n\r\nApprove", "\r\n\r\nPlease approve"),
            raw.replace("From:", "Auto-Submitted: auto-replied\r\nFrom:"),
            raw.replace("In-Reply-To:", "References:"),
            raw.replace("Moe <moe@example.com>", "moe@example.com, evil@example.com"),
        ] {
            assert!(parse_reply(bad.as_bytes()).is_none(), "{bad}");
        }
    }
    #[tokio::test]
    async fn imap_uses_peek_and_resumes_after_checkpoint_without_deleting_mail() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let state = AppState::new(
            crate::store::Store::open(":memory:", [8; 32]).unwrap(),
            "https://relay.example".into(),
            crate::providers::Providers::development().unwrap(),
        );
        let inbox = Inbox {
            host: "imap.example".into(),
            port: 993,
            user: "inbox".into(),
            password: "test-password".into(),
        };
        for expected_fetches in [1, 0] {
            let (client, server) = tokio::io::duplex(16384);
            let server = tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(server);
                let mut read = BufReader::new(read);
                write.write_all(b"* OK Test IMAP ready\r\n").await.unwrap();
                let mut fetches = 0;
                loop {
                    let mut line = String::new();
                    if read.read_line(&mut line).await.unwrap() == 0 {
                        break;
                    }
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    let response = if command.starts_with("LOGIN ") {
                        format!("{tag} OK LOGIN completed\r\n")
                    } else if command.starts_with("SELECT ") {
                        format!(
                            "* 1 EXISTS\r\n* OK [UIDVALIDITY 77] UIDs valid\r\n{tag} OK [READ-WRITE] SELECT completed\r\n"
                        )
                    } else if command.starts_with("UID SEARCH ") {
                        format!("* SEARCH 1\r\n{tag} OK SEARCH completed\r\n")
                    } else if command.starts_with("UID FETCH ") {
                        assert!(command.contains("BODY.PEEK[]"));
                        fetches += 1;
                        let raw = "From: unknown@example.com\r\n\r\nNot an approval";
                        format!(
                            "* 1 FETCH (UID 1 BODY[] {{{}}}\r\n{})\r\n{tag} OK FETCH completed\r\n",
                            raw.len(),
                            raw
                        )
                    } else if command == "LOGOUT" {
                        format!("* BYE logout\r\n{tag} OK LOGOUT completed\r\n")
                    } else {
                        panic!("Unexpected IMAP operation: {command}")
                    };
                    write.write_all(response.as_bytes()).await.unwrap();
                    if command == "LOGOUT" {
                        break;
                    }
                }
                fetches
            });
            tokio::time::timeout(Duration::from_secs(3), poll_stream(&state, &inbox, client))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(server.await.unwrap(), expected_fetches);
        }
    }
}
