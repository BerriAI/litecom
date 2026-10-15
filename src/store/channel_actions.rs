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
}
#[derive(Serialize, Deserialize)]
enum Target {
    Request { version: u32, digest: String },
    Permission { id: String, digest: String },
}

impl Store {
    pub(crate) fn channel_action(&self, d: &Delivery) -> Result<Option<ChannelAction>> {
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
        let (target, body) = if task.view.status == Status::AwaitingApproval {
            let draft = task.view.draft.as_ref().ok_or(Error::NotFound)?;
            let Some(action) = &draft.action else {
                // An attachment snapshot requires an actual preview; do not turn its name into consent.
                return Ok(None);
            };
            let description = match action {
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
            (
                Target::Request {
                    version: draft.version,
                    digest: draft.digest.clone(),
                },
                format!(
                    "{} asks:\n\"{}\"\n\n{description}",
                    requester.name, task.view.request
                ),
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
        let target_key = hash(serde_json::to_vec(&target)?);
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
        };
        self.db.execute("INSERT INTO channel_actions(token_hash,code,channel_id,task_id,target_key,data) VALUES(?,?,?,?,?,?)",
            params![hash(&token),code,d.channel_id,d.task_id,target_key,self.vault.seal(&record,&format!("channel-action:{}:{}:{target_key}",d.channel_id,d.task_id))?])?;
        Ok(Some(ChannelAction { token, code, body }))
    }

    pub(crate) fn channel_decide(
        &self,
        actor: &ChannelActor,
        key: &str,
        decision: Decision,
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
        require(
            verified
                && actor.key() == record.actor
                && actor.key() == ChannelActor::from_destination(&kind, &destination)?.key(),
            "This approval is not for this sender.",
        )?;
        let p = Principal {
            user_id: user,
            role: Role::Human,
            agent_id: None,
            credential_id: format!("channel:{channel}"),
        };
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
                self.decide(&p, &task, version, &digest, decision.clone())?;
            }
            Target::Permission { id, digest } => {
                self.decide_permission(&p, &task, &id, &digest, decision.clone())?;
            }
        }
        Ok(ReplyOutcome::Decided(decision))
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
