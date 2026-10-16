use super::*;
use crate::slack::SlackDestination;
use crate::slack_agent::{Job, Mention};

pub(crate) struct SlackLink {
    pub channel_id: String,
    pub user_id: String,
    pub destination: SlackDestination,
}

impl Store {
    pub(crate) fn slack_links(&self, team: &str) -> Result<Vec<SlackLink>> {
        let rows = self.db.prepare("SELECT id,user_id,destination FROM channels WHERE kind='slack' AND verified=1 ORDER BY id")?
            .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?)))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        let mut links = Vec::new();
        for (channel_id, user_id, bytes) in rows {
            let raw: String = self.vault.open(&bytes, &format!("channel:{channel_id}"))?;
            let destination: SlackDestination = serde_json::from_str(&raw)?;
            if destination.team == team {
                links.push(SlackLink {
                    channel_id,
                    user_id,
                    destination,
                });
            }
        }
        Ok(links)
    }

    pub(crate) fn slack_principal(&self, team: &str, user: &str) -> Result<Option<Principal>> {
        let links = self
            .slack_links(team)?
            .into_iter()
            .filter(|l| l.destination.user == user)
            .collect::<Vec<_>>();
        conflict(
            links.len() <= 1,
            "This Slack identity is linked to multiple accounts. Reconnect Slack to one account.",
        )?;
        Ok(links.into_iter().next().map(|l| Principal {
            user_id: l.user_id,
            role: Role::Agent,
            agent_id: None,
            credential_id: format!("slack:{}:{}", team, user),
        }))
    }

    pub(crate) fn enqueue_slack(&self, mention: Mention) -> Result<()> {
        let actor = hash(format!("slack:{}:{}", mention.team, mention.user));
        let job = Job::new(mention);
        self.transaction(|| {
            if self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM slack_jobs WHERE id=?)",
                [&job.id],
                |r| r.get::<_, bool>(0),
            )? {
                return Ok(());
            }
            self.slack_capacity(&actor)?;
            self.db.execute(
                "INSERT OR IGNORE INTO slack_jobs(id,data) VALUES(?,?)",
                params![
                    job.id,
                    self.vault.seal(&job, &format!("slack-job:{}", job.id))?
                ],
            )?;
            Ok(())
        })
    }

    pub(crate) fn claim_slack(&self) -> Result<Option<Job>> {
        self.transaction(|| {
            let row = self.db.query_row("SELECT id,data FROM slack_jobs WHERE done=0 AND next_attempt<=? AND claim_until<=? ORDER BY next_attempt,rowid LIMIT 1",
                params![self.time(), self.time()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?))).optional()?;
            let Some((id, bytes)) = row else { return Ok(None) };
            let mut job: Job = self.vault.open(&bytes,&format!("slack-job:{id}"))?;
            job.claim = secret();
            self.db.execute("UPDATE slack_jobs SET claim=?,claim_until=? WHERE id=?",params![job.claim,self.time()+120,id])?;
            Ok(Some(job))
        })
    }

    pub(crate) fn save_slack(&self, job: &Job, done: bool, delay: i64) -> Result<()> {
        let changed = self.db.execute("UPDATE slack_jobs SET data=?,done=?,next_attempt=?,claim=NULL,claim_until=0,attempts=0 WHERE id=? AND claim=?",
            params![self.vault.seal(job,&format!("slack-job:{}",job.id))?,done,self.time()+delay,job.id,job.claim])?;
        conflict(
            changed == 1,
            "This Slack event is being handled by another worker.",
        )
    }

    pub(crate) fn retry_slack(&self, job: &Job) -> Result<()> {
        self.db.execute("UPDATE slack_jobs SET claim=NULL,claim_until=0,attempts=attempts+1,next_attempt=?+MIN(3600,30*(1 << MIN(attempts,7))) WHERE id=? AND claim=?",
            params![self.time(),job.id,job.claim])?;
        Ok(())
    }

    pub(crate) fn checkpoint_slack(&self, job: &Job) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE slack_jobs SET data=?,claim_until=? WHERE id=? AND claim=?",
            params![
                self.vault.seal(job, &format!("slack-job:{}", job.id))?,
                self.time() + 120,
                job.id,
                job.claim
            ],
        )?;
        conflict(
            changed == 1,
            "This Slack event is being handled by another worker.",
        )
    }
}
