//! Channel capabilities identify an exact proposal; they never act as agent credentials.
use super::*;
use crate::{
    channel_replies::{ChannelAction, ChannelActor, ReplyOutcome},
    notifications::Delivery,
};

#[derive(Serialize, Deserialize)]
struct Record {
    token: String,
    actor: String,
    target: Target,
    #[serde(default)]
    slack_channel: Option<String>,
}
#[derive(Serialize, Deserialize)]
enum Target {
    Request { version: u32, digest: String },
    Permission { id: String, digest: String },
}

pub(crate) struct SlackThreadCard {
    pub summary: String,
    pub recipient: Option<String>,
    pub action: Option<ChannelAction>,
}

impl Store {
    pub(crate) fn slack_thread_card(
        &self,
        job: &crate::slack_agent::Job,
    ) -> Result<SlackThreadCard> {
        let task_id = job.task.as_deref().ok_or(Error::NotFound)?;
        // Bind this surface to the durable signed event, never model arguments.
        let data: Vec<u8> =
            self.db
                .query_row("SELECT data FROM slack_jobs WHERE id=?", [&job.id], |r| {
                    r.get(0)
                })?;
        let saved: crate::slack_agent::Job =
            self.vault.open(&data, &format!("slack-job:{}", job.id))?;
        require(
            saved.task.as_deref() == Some(task_id)
                && saved.mention.team == job.mention.team
                && saved.mention.channel == job.mention.channel
                && saved.mention.thread_ts == job.mention.thread_ts,
            "This request does not belong to this Slack thread.",
        )?;
        let task = self.task(task_id)?;
        require(
            saved.requester.as_deref() == Some(&task.view.requester_id),
            "This request does not belong to this Slack sender.",
        )?;
        let source = task.source.as_ref().ok_or(Error::NotFound)?;
        require(source.url == saved.source, "The request source changed.")?;
        let requester = self.profile_by_id(&task.view.requester_id)?;
        let mut card = SlackThreadCard {
            summary: format!(
                "Ask: {}\n\nFrom: {}\n\nWhere: [Slack thread]({})",
                source.ask, requester.name, source.url
            ),
            recipient: None,
            action: None,
        };
        let Some(link) = self
            .slack_links(&saved.mention.team)?
            .into_iter()
            .find(|l| l.user_id == task.view.owner_id)
        else {
            return Ok(card);
        };
        require(
            self.slack_principal(&link.destination.team, &link.destination.user)?
                .is_some_and(|p| p.user_id == task.view.owner_id),
            "The recipient's Slack connection changed.",
        )?;
        card.recipient = Some(link.destination.user.clone());
        // Custom/changed proposals and tool permissions need their private
        // review surface; do not expose their instructions in a public thread.
        let standard = task
            .view
            .draft
            .as_ref()
            .and_then(|d| d.action.as_ref())
            .is_some_and(|action| match action {
                Action::Harness { instruction, .. }
                | Action::Cloud { instruction, .. }
                | Action::ReadOnly { instruction, .. } => instruction == &task.view.request,
                _ => false,
            });
        if task.view.status == Status::AwaitingApproval && standard {
            card.action = self.channel_action_on(
                &Delivery {
                    event_id: 0,
                    channel_id: link.channel_id,
                    destination: serde_json::to_string(&link.destination)?,
                    task_id: task_id.into(),
                    kind: "awaiting_approval".into(),
                    attempts: 0,
                    channel_kind: "slack".into(),
                    previous: None,
                },
                Some(&saved.mention.channel),
            )?;
        }
        Ok(card)
    }
    pub(crate) fn notification_summary(&self, d: &Delivery) -> Result<String> {
        let task = self.task(&d.task_id)?;
        let requester = self.profile_by_id(&task.view.requester_id)?;
        Ok(if let Some(source) = &task.source {
            format!(
                "Ask: {}\n\nFrom: {}\n\nWhere: [Slack thread]({})",
                source.ask, requester.name, source.url
            )
        } else {
            format!(
                "Ask: {}\n\nFrom: {}\n\nWhere: Litecom",
                task.view.request, requester.name
            )
        })
    }
    pub(crate) fn channel_action(&self, d: &Delivery) -> Result<Option<ChannelAction>> {
        self.channel_action_on(d, None)
    }

    fn channel_action_on(
        &self,
        d: &Delivery,
        slack_channel: Option<&str>,
    ) -> Result<Option<ChannelAction>> {
        let task = self.task(&d.task_id)?;
        let (owner, kind, destination, verified): (String, String, Vec<u8>, bool) =
            self.db.query_row(
                "SELECT user_id,kind,destination,verified FROM channels WHERE id=?",
                [&d.channel_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        if owner != task.view.owner_id
            || !verified
            || task.view.expired(self.time())
            || task.view.revoked_at.is_some()
        {
            return Ok(None);
        }
        let destination: String = self
            .vault
            .open(&destination, &format!("channel:{}", d.channel_id))?;
        // A queued delivery for an old destination cannot create approval authority.
        if destination != d.destination {
            return Ok(None);
        }
        let requester = self.profile_by_id(&task.view.requester_id)?;
        let read_only = task.view.status == Status::AwaitingApproval
            && task
                .view
                .draft
                .as_ref()
                .and_then(|d| d.action.as_ref())
                .is_some_and(|a| a.as_read_only().is_ok());
        let actions = !task.view.ask_only;
        let (target, body) = if task.view.status == Status::AwaitingApproval {
            let draft = task.view.draft.as_ref().ok_or(Error::NotFound)?;
            let Some(action) = &draft.action else {
                // An attachment snapshot requires an actual preview; do not turn its name into consent.
                return Ok(None);
            };
            let description = match action {
                Action::ReadOnly { workspace, .. } => format!(
                    "Read local files, GitHub and Slack, then send the answer. No actions. Folder: {workspace}"
                ),
                Action::Harness {
                    title,
                    description,
                    instruction,
                    workspace,
                    acting_account,
                    ..
                } => {
                    format!(
                        "{title}. {description}\nAccount: {acting_account}\nFolder: {workspace}{}",
                        if instruction != &task.view.request {
                            format!("\nAgent instruction: {instruction}")
                        } else {
                            String::new()
                        }
                    )
                }
                Action::Cloud {
                    title,
                    description,
                    instruction,
                    acting_account,
                    ..
                } => {
                    format!(
                        "{title}. {description}\nAccount: {acting_account}{}",
                        if instruction != &task.view.request {
                            format!("\nAgent instruction: {instruction}")
                        } else {
                            String::new()
                        }
                    )
                }
                Action::Command { .. } => serde_json::to_string_pretty(action)?,
            };
            let body = if read_only
                && matches!(action,Action::Harness {instruction,..} | Action::ReadOnly {instruction,..} if instruction==&task.view.request)
            {
                let access = if task.view.ask_only {
                    "Read-only: read local files, GitHub and Slack. No actions."
                } else {
                    "Choose read-only for an answer, or approve actions to allow changes to files and connected apps."
                };
                let delivery = if task.source.as_ref().is_some_and(|s| s.share_files) {
                    "The answer and requested files are shared with everyone in the source Slack channel."
                } else if task.source.is_some() {
                    "The answer is shared in the source Slack channel. File downloads stay restricted to you and the requester."
                } else {
                    "The answer and requested files are sent to the requester."
                };
                format!("{}\n\n{access}\n{delivery}", self.notification_summary(d)?)
            } else if let Some(source) = &task.source {
                let delivery = if source.share_files {
                    "reply and files"
                } else {
                    "reply"
                };
                let mut body = format!(
                    "{}\n\nApprove to run your agent and share its {delivery} with everyone in that thread.",
                    self.notification_summary(d)?
                );
                // A changed proposal still needs its exact additional instruction.
                match action {
                    Action::Harness { instruction, .. } | Action::Cloud { instruction, .. }
                        if instruction == &task.view.request => {}
                    _ => body.push_str(&format!("\n\n{description}")),
                }
                body
            } else {
                format!(
                    "Ask: {}\n\nFrom: {}\n\nWhere: Litecom\n\n{description}",
                    task.view.request, requester.name
                )
            };
            (
                Target::Request {
                    version: draft.version,
                    digest: draft.digest.clone(),
                },
                body,
            )
        } else if task.view.status == Status::Executing {
            let Some(p) = self.pending_permissions(&d.task_id)?.into_iter().next() else {
                return Ok(None);
            };
            (
                Target::Permission {
                    id: p.id,
                    digest: p.digest,
                },
                format!(
                    "For {}'s request, your agent needs one additional permission:\n{}\n{}\n\nApprove allows this operation once.",
                    requester.name,
                    p.title,
                    serde_json::to_string_pretty(&p.details)?
                ),
            )
        } else {
            return Ok(None);
        };
        let target_key = if let Some(channel) = slack_channel {
            hash(serde_json::to_vec(&(&target, channel))?)
        } else {
            hash(serde_json::to_vec(&target)?)
        };
        let old: Option<(String,Vec<u8>)> = self.db.query_row(
            "SELECT code,data FROM channel_actions WHERE channel_id=? AND task_id=? AND target_key=?",
            params![d.channel_id,d.task_id,target_key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((code, data)) = old {
            let record: Record = self.vault.open(
                &data,
                &format!("channel-action:{}:{}:{target_key}", d.channel_id, d.task_id),
            )?;
            return Ok(Some(ChannelAction {
                token: record.token,
                code,
                body,
                read_only,
                actions,
            }));
        }
        let token = secret();
        let code = loop {
            let candidate = hex::encode(crate::crypto::random_bytes::<4>()).to_uppercase();
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM channel_actions WHERE code=?)",
                [&candidate],
                |r| r.get(0),
            )?;
            if !exists {
                break candidate;
            }
        };
        let actor = ChannelActor::from_destination(&kind, &destination)?.key();
        let record = Record {
            token: token.clone(),
            actor,
            target,
            slack_channel: slack_channel.map(str::to_owned),
        };
        self.db.execute("INSERT INTO channel_actions(token_hash,code,channel_id,task_id,target_key,data) VALUES(?,?,?,?,?,?)",
            params![hash(&token),code,d.channel_id,d.task_id,target_key,self.vault.seal(&record,&format!("channel-action:{}:{}:{target_key}",d.channel_id,d.task_id))?])?;
        Ok(Some(ChannelAction {
            token,
            code,
            body,
            read_only,
            actions,
        }))
    }

    #[cfg(test)]
    pub(crate) fn channel_decide(
        &self,
        actor: &ChannelActor,
        key: &str,
        decision: Decision,
    ) -> Result<ReplyOutcome> {
        self.channel_decide_access(actor, key, decision, false)
    }
    pub(crate) fn channel_decide_access(
        &self,
        actor: &ChannelActor,
        key: &str,
        decision: Decision,
        read_only: bool,
    ) -> Result<ReplyOutcome> {
        let (query, key) = if matches!(actor, ChannelActor::Phone(_)) {
            ("a.code=?", key.to_ascii_uppercase())
        } else {
            ("a.token_hash=?", hash(key))
        };
        let row = self.db.query_row(&format!("SELECT a.channel_id,a.task_id,a.target_key,a.data,c.user_id,c.kind,c.destination,c.verified FROM channel_actions a JOIN channels c ON c.id=a.channel_id WHERE {query}"),[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Vec<u8>>(6)?,r.get::<_,bool>(7)?))).optional()?.ok_or(Error::NotFound)?;
        let (channel, task, target_key, data, user, kind, destination, verified) = row;
        let record: Record = self.vault.open(
            &data,
            &format!("channel-action:{channel}:{task}:{target_key}"),
        )?;
        let destination: String = self
            .vault
            .open(&destination, &format!("channel:{channel}"))?;
        let mut expected = ChannelActor::from_destination(&kind, &destination)?;
        let linked_actor = expected.key();
        if let Some(source_channel) = &record.slack_channel {
            match &mut expected {
                ChannelActor::Slack { channel, .. } => *channel = source_channel.clone(),
                _ => return Err(Error::Forbidden("This approval requires Slack.".into())),
            }
        }
        require(
            verified && linked_actor == record.actor && actor.key() == expected.key(),
            "This approval is not for this sender.",
        )?;
        let p = Principal {
            user_id: user,
            role: Role::Human,
            agent_id: None,
            credential_id: format!("channel:{channel}"),
        };
        self.check_workspace(&p.user_id)?;
        let previous:Option<String>=match &record.target {
            Target::Request {version,digest}=> self.db.query_row("SELECT decision FROM approvals WHERE task_id=? AND version=? AND digest=? AND user_id=?",params![task,version,digest,p.user_id],|r|r.get(0)).optional()?,
            Target::Permission {id,..}=> self.db.query_row("SELECT decision FROM tool_permissions WHERE task_id=? AND id=?",params![task,id],|r|r.get(0)).optional()?.flatten(),
        };
        if let Some(previous) = previous {
            let previous: Decision = serde_json::from_str(&previous)?;
            return Ok(ReplyOutcome::AlreadyDecided(previous));
        }
        match record.target {
            Target::Request { version, digest } => {
                self.decide_access(&p, &task, version, &digest, decision.clone(), read_only)?;
            }
            Target::Permission { id, digest } => {
                require(
                    !read_only,
                    "A tool permission cannot change the run's access. Send a new read-only request.",
                )?;
                self.decide_permission(&p, &task, &id, &digest, decision.clone())?;
            }
        }
        Ok(ReplyOutcome::Decided(decision))
    }

    pub(crate) fn phone_notifications_enabled(&self, phone: &str) -> Result<bool> {
        let key = self.vault.fingerprint("phone", phone.as_bytes());
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM channels c JOIN profiles p ON p.id=c.user_id WHERE p.phone_key=? AND c.kind='phone' AND c.verified=1)", [key], |r| r.get(0))?)
    }

    pub(crate) fn phone_event_seen(&self, body_hash: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM phone_events WHERE body_hash=?)",
            [body_hash],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn record_phone_event(&self, body_hash: &str) -> Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO phone_events(body_hash,created_at) VALUES(?,?)",
            params![body_hash, self.time()],
        )?;
        Ok(())
    }

    pub(crate) fn stop_phone_notifications(&self, phone: &str) -> Result<()> {
        let mut found = None;
        let rows = self
            .db
            .prepare(
                "SELECT id,user_id,destination FROM channels WHERE kind='phone' AND verified=1",
            )?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, user, data) in rows {
            let destination: String = self.vault.open(&data, &format!("channel:{id}"))?;
            if destination == phone {
                found = Some((id, user));
                break;
            }
        }
        if let Some((id, user_id)) = found {
            self.disable_channel(
                &Principal {
                    user_id,
                    role: Role::Human,
                    agent_id: None,
                    credential_id: format!("channel:{id}"),
                },
                &id,
            )?;
        }
        Ok(())
    }
}
