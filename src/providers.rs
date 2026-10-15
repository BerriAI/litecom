use crate::error::{Error, Result};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use serde_json::Value;

#[derive(Clone)]
pub struct Providers {
    client: reqwest::Client,
    twilio: Option<Twilio>,
    email: Option<Email>,
    pub slack: Option<crate::slack::Slack>,
    pub dev: bool,
    slack_signing: Option<String>,
    pub(crate) imap: Option<crate::email_replies::Inbox>,
}
#[derive(Clone)]
struct Twilio {
    sid: String,
    token: String,
    service: String,
}
#[derive(Clone)]
struct Email {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    reply_to: Option<Mailbox>,
}
impl Providers {
    pub fn development() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| Error::Internal(e.into()))?,
            twilio: None,
            email: None,
            slack: None,
            dev: true,
            slack_signing: None,
            imap: None,
        })
    }
    pub fn from_env(dev: bool) -> Result<Self> {
        let get = |key| std::env::var(key).ok().filter(|s| !s.trim().is_empty());
        let twilio=match (get("TWILIO_ACCOUNT_SID"),get("TWILIO_AUTH_TOKEN"),get("TWILIO_MESSAGING_SERVICE_SID")) {
            (Some(sid),Some(token),Some(service))=>Some(Twilio{sid,token,service}),
            (None,None,None)=>None,
            _=>return Err(Error::Unavailable("Set all three Twilio credentials: account SID, auth token, and messaging service SID.".into())),
        };
        let email = match (get("SMTP_HOST"), get("SMTP_FROM")) {
            (None, None) => None,
            (Some(host), Some(from)) => {
                let port = get("SMTP_PORT")
                    .unwrap_or_else(|| "465".into())
                    .parse::<u16>()
                    .map_err(|_| Error::Invalid("SMTP_PORT must be a valid port number.".into()))?;
                let transport = if port == 465 {
                    AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
                } else {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
                }
                .map_err(|_| Error::Invalid("Check SMTP_HOST and its TLS configuration.".into()))?;
                let mut transport = transport
                    .port(port)
                    .timeout(Some(std::time::Duration::from_secs(20)));
                match (get("SMTP_USERNAME"), get("SMTP_PASSWORD")) {
                    (Some(user), Some(password)) => {
                        transport = transport.credentials(Credentials::new(user, password))
                    }
                    (None, None) => {}
                    _ => {
                        return Err(Error::Invalid(
                            "Set both SMTP_USERNAME and SMTP_PASSWORD.".into(),
                        ));
                    }
                }
                Some(Email {
                    transport: transport.build(),
                    reply_to: get("SMTP_REPLY_TO")
                        .map(|v| {
                            v.parse::<Mailbox>().map_err(|_| {
                                Error::Invalid("SMTP_REPLY_TO must be an email address.".into())
                            })
                        })
                        .transpose()?,
                    from: from.parse().map_err(|_| {
                        Error::Invalid("SMTP_FROM must be a valid sender address.".into())
                    })?,
                })
            }
            _ => return Err(Error::Invalid("Set both SMTP_HOST and SMTP_FROM.".into())),
        };
        let slack = match (get("SLACK_CLIENT_ID"), get("SLACK_CLIENT_SECRET")) {
            (Some(client_id), Some(client_secret)) => {
                Some(crate::slack::Slack::new(client_id, client_secret))
            }
            (None, None) => None,
            _ => {
                return Err(Error::Invalid(
                    "Set both SLACK_CLIENT_ID and SLACK_CLIENT_SECRET.".into(),
                ));
            }
        };
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| Error::Internal(e.into()))?,
            twilio,
            email,
            slack,
            dev,
            slack_signing: get("SLACK_SIGNING_SECRET"),
            imap: crate::email_replies::Inbox::from_env()?,
        })
    }
    pub(crate) fn twilio_verification(&self) -> Option<(&str, &str)> {
        if self.dev {
            return None;
        }
        self.twilio
            .as_ref()
            .map(|p| (p.sid.as_str(), p.token.as_str()))
    }
    pub(crate) fn slack_signing_secret(&self) -> Option<&str> {
        if self.dev {
            None
        } else {
            self.slack_signing.as_deref()
        }
    }
    pub(crate) fn email_replies_available(&self) -> bool {
        !self.dev && self.imap.is_some()
    }
    #[cfg(test)]
    pub(crate) fn test_replies(twilio_sid: &str, twilio_token: &str, slack_secret: &str) -> Self {
        let mut value = Self::development().unwrap();
        value.dev = false;
        value.twilio = Some(Twilio {
            sid: twilio_sid.into(),
            token: twilio_token.into(),
            service: String::new(),
        });
        value.slack_signing = Some(slack_secret.into());
        value
    }
    pub fn phone_available(&self) -> bool {
        self.dev || self.twilio.is_some()
    }
    pub fn email_available(&self) -> bool {
        self.dev || self.email.is_some()
    }
    pub fn http(&self) -> &reqwest::Client {
        &self.client
    }
    pub async fn email(&self, to: &str, subject: &str, body: &str) -> Result<String> {
        self.email_notice(to, subject, body, None).await
    }
    pub(crate) async fn email_notice(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        action: Option<&str>,
    ) -> Result<String> {
        let to = crate::channels::email_address(to)?;
        if self.dev {
            eprintln!("[LOCAL DEVELOPMENT — no email sent] {to}: {body}");
            return Ok("local-development".into());
        }
        let provider = self.email.as_ref().ok_or_else(|| {
            Error::Unavailable("Email delivery is not enabled on this server.".into())
        })?;
        let local = action
            .map(|token| format!("litecom-approve-{token}"))
            .unwrap_or_else(crate::crypto::id);
        let message_id = format!("<{local}@{}>", provider.from.email.domain());
        let mut builder = Message::builder()
            .from(provider.from.clone())
            .to(to
                .parse()
                .map_err(|_| Error::Invalid("Enter a valid email address.".into()))?)
            .subject(subject)
            .message_id(Some(message_id.clone()));
        if let Some(reply_to) = &provider.reply_to {
            builder = builder.reply_to(reply_to.clone());
        }
        let message = builder
            .body(body.to_owned())
            .map_err(|_| Error::Invalid("The email could not be prepared.".into()))?;
        provider.transport.send(message).await.map_err(|_|Error::Unavailable("The mail server did not accept this message. Try again or ask its operator to check delivery.".into()))?;
        Ok(message_id)
    }
    pub async fn sms(&self, to: &str, body: &str) -> Result<String> {
        if self.dev {
            eprintln!("[LOCAL DEVELOPMENT — no SMS sent] {to}: {body}");
            return Ok("local-development".into());
        }
        let provider=self.twilio.as_ref().ok_or_else(||Error::Unavailable("Phone delivery is not enabled on this server. Ask its owner to connect a messaging provider.".into()))?;
        let result = self
            .client
            .post(format!(
                "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
                provider.sid
            ))
            .basic_auth(&provider.sid, Some(&provider.token))
            .form(&[
                ("To", to),
                ("MessagingServiceSid", provider.service.as_str()),
                ("Body", body),
            ])
            .send()
            .await
            .map_err(|_| {
                Error::Unavailable(
                    "The phone provider could not be reached. Try again shortly.".into(),
                )
            })?;
        if !result.status().is_success() {
            return Err(Error::Unavailable("The phone provider did not accept this message. Check the number or ask the server owner to inspect delivery settings.".into()));
        }
        let body: Value = result.json().await.map_err(|e| Error::Internal(e.into()))?;
        body["sid"].as_str().map(str::to_owned).ok_or_else(|| {
            Error::Unavailable("The phone provider returned an incomplete response.".into())
        })
    }
}
