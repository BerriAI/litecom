//! Durable relay-owned jobs. Credentials never appear in public agent records.
use super::*;
use crate::devin::{Connection as DevinConnection, Session};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CloudJob {
    pub principal: Principal,
    pub work: WorkItem,
    pub connection: DevinConnection,
    pub session: Option<Session>,
}
pub(crate) struct CloudClaim {
    pub job: CloudJob,
    pub phase: String,
    pub token: String,
}

impl Store {
    pub fn connect_devin(
        &self,
        p: &Principal,
        name: &str,
        mut connection: DevinConnection,
    ) -> Result<Agent> {
        p.human()?;
        validate(!name.trim().is_empty(), "Give this agent a name.")?;
        self.transaction(|| {
            let existing = self
                .db
                .query_row(
                    "SELECT * FROM agents WHERE owner_id=? AND name=? COLLATE NOCASE",
                    params![p.user_id, name.trim()],
                    Self::agent_row,
                )
                .optional()?;
            let agent = if let Some(agent) = existing {
                conflict(
                    agent.kind == AgentKind::Devin && !agent.revoked,
                    "That name is already in use. Choose another name.",
                )?;
                if let Some(old) = self.devin_connection(&agent.id)?
                    && old.token == connection.token
                    && old.org_id == connection.org_id
                    && old.principal_id == connection.principal_id
                {
                    // A repeated submit or lost HTTP response does not invalidate approvals.
                    connection.revision = old.revision;
                }
                agent
            } else {
                let agent = Agent {
                    id: id(),
                    owner_id: p.user_id.clone(),
                    name: name.trim().into(),
                    kind: AgentKind::Devin,
                    is_default: false,
                    revoked: false,
                    last_seen_at: None,
                    connected: false,
                    local: None,
                };
                self.insert_agent(&agent)?;
                agent
            };
            self.db.execute(
                "UPDATE agents SET config=? WHERE id=?",
                params![
                    self.vault
                        .seal(&connection, &format!("agent-config:{}", agent.id))?,
                    agent.id
                ],
            )?;
            Ok(agent)
        })
    }
    fn devin_connection(&self, agent: &str) -> Result<Option<DevinConnection>> {
        let data: Option<Vec<u8>> =
            self.db
                .query_row("SELECT config FROM agents WHERE id=?", [agent], |r| {
                    r.get(0)
                })?;
        data.map(|data| self.vault.open(&data, &format!("agent-config:{agent}")))
            .transpose()
    }
    /// Runs only in the relay; no agent API can issue this internal principal.
    pub(crate) fn queue_cloud_work(&self) -> Result<()> {
        let agents = self
            .db
            .prepare(
                "SELECT * FROM agents WHERE kind='\"devin\"' AND revoked=0 AND config IS NOT NULL",
            )?
            .query_map([], Self::agent_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for agent in agents {
            match self.check_workspace(&agent.owner_id) {
                Ok(()) => {}
                Err(Error::Forbidden(_)) => continue,
                Err(error) => return Err(error),
            }
            let Some(connection) = self.devin_connection(&agent.id)? else {
                continue;
            };
            let p = Principal {
                user_id: agent.owner_id,
                role: Role::Agent,
                agent_id: Some(agent.id.clone()),
                credential_id: format!("relay-cloud:{}", agent.id),
            };
            let Some(work) = self.claim_internal(&p, &agent.id)? else {
                continue;
            };
            let action = connection.action(&work.request);
            if work.phase == "prepare" {
                self.prepare(
                    &p,
                    &work.task_id,
                    Some(&work.lease),
                    DraftInput {
                        text: String::new(),
                        attachment_ids: vec![],
                        action: Some(action),
                    },
                )?;
            } else if work
                .draft
                .as_ref()
                .and_then(|d| d.action.as_ref())
                .map(serde_json::to_value)
                .transpose()?
                != Some(serde_json::to_value(&action)?)
            {
                self.finish(&p, &work.task_id, &work.lease, Receipt {outcome:Outcome::Failed,summary:"The Devin connection or approved operation changed. Send a new request and review the current connection before running it.".into(),urls:vec![]})?;
            } else {
                let job = CloudJob {
                    principal: p,
                    work,
                    connection,
                    session: None,
                };
                // A crash before this insert leaves an uncertain execution lease.
                // A crash after it is recoverable without reissuing a session POST.
                self.db.execute(
                    "INSERT INTO cloud_jobs(task_id,phase,data) VALUES(?,'ready',?)",
                    params![
                        job.work.task_id,
                        self.vault
                            .seal(&job, &format!("cloud-job:{}", job.work.task_id))?
                    ],
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn claim_cloud_job(&self) -> Result<Option<CloudClaim>> {
        self.recover()?;
        self.transaction(|| {
            let row = self.db.query_row("SELECT task_id,phase,data FROM cloud_jobs WHERE phase IN ('ready','starting','running') AND claim_until<=? AND next_attempt<=? ORDER BY next_attempt,task_id LIMIT 1",params![self.time(),self.time()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?))).optional()?;
            let Some((task_id, phase, data)) = row else {return Ok(None)};
            let job:CloudJob = self.vault.open(&data,&format!("cloud-job:{task_id}"))?;
            let token = secret();
            self.db.execute("UPDATE cloud_jobs SET phase=?,claim_hash=?,claim_until=? WHERE task_id=?",params![if phase=="ready" {"starting"} else {&phase}, hash(&token),self.time()+LEASE_TTL,task_id])?;
            let claim=CloudClaim {job,phase,token};
            self.renew_cloud_inner(&claim)?;
            Ok(Some(claim))
        })
    }
    fn check_cloud_claim(&self, claim: &CloudClaim) -> Result<()> {
        let valid:bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM cloud_jobs WHERE task_id=? AND claim_hash=? AND claim_until>?)",params![claim.job.work.task_id,hash(&claim.token),self.time()],|r|r.get(0))?;
        conflict(valid, "Another relay worker is monitoring this session.")
    }
    fn renew_cloud_inner(&self, claim: &CloudClaim) -> Result<Status> {
        self.check_cloud_claim(claim)?;
        let mut task = self.task(&claim.job.work.task_id)?;
        if matches!(
            task.view.status,
            Status::Executing | Status::CancelRequested
        ) {
            if task.view.expired(self.time()) || !self.task_workspace_allowed(&task)? {
                task.view.status = Status::CancelRequested;
            }
            task.lease_until = Some(self.time() + LEASE_TTL);
            self.db.execute(
                "UPDATE tasks SET status=?,data=? WHERE id=?",
                params![
                    task.view.status.as_str(),
                    self.vault.seal(&task, &format!("task:{}", task.view.id))?,
                    task.view.id
                ],
            )?;
        }
        self.db.execute(
            "UPDATE cloud_jobs SET claim_until=? WHERE task_id=?",
            params![self.time() + LEASE_TTL, task.view.id],
        )?;
        Ok(task.view.status)
    }
    pub(crate) fn renew_cloud(&self, claim: &CloudClaim) -> Result<Status> {
        self.transaction(|| self.renew_cloud_inner(claim))
    }
    pub(crate) fn cloud_started(&self, claim: &mut CloudClaim, session: Session) -> Result<()> {
        self.check_cloud_claim(claim)?;
        claim.job.session = Some(session);
        self.db.execute(
            "UPDATE cloud_jobs SET phase='running',data=? WHERE task_id=?",
            params![
                self.vault
                    .seal(&claim.job, &format!("cloud-job:{}", claim.job.work.task_id))?,
                claim.job.work.task_id
            ],
        )?;
        Ok(())
    }
    pub(crate) fn cloud_progress(
        &self,
        claim: &CloudClaim,
        message: &str,
        attention: bool,
    ) -> Result<()> {
        self.transaction(|| {
            self.check_cloud_claim(claim)?;
            let mut task = self.task(&claim.job.work.task_id)?;
            let progress = ExecutionProgress {
                message: message.into(),
                needs_attention: attention,
                url: claim
                    .job
                    .session
                    .as_ref()
                    .and_then(|s| crate::devin::owner_session_url(&s.url)),
            };
            if task.view.execution.as_ref() != Some(&progress) {
                task.view.execution = Some(progress);
                self.save(
                    &mut task,
                    if attention {
                        "cloud_attention"
                    } else {
                        "cloud_progress"
                    },
                )?;
            }
            Ok(())
        })
    }
    pub(crate) fn release_cloud(&self, claim: &CloudClaim, finished: bool) -> Result<()> {
        self.check_cloud_claim(claim)?;
        if finished {
            // Completed jobs no longer need a copy of the provider credential.
            self.db.execute(
                "DELETE FROM cloud_jobs WHERE task_id=?",
                [&claim.job.work.task_id],
            )?;
        } else {
            self.db.execute("UPDATE cloud_jobs SET claim_hash=NULL,claim_until=0,next_attempt=? WHERE task_id=?",params![self.time()+10,claim.job.work.task_id])?;
        }
        Ok(())
    }
    /// May record a cancellation after agent revocation or task expiry. External
    /// workers cannot use this route to bypass their normal live-lease checks.
    pub(crate) fn cloud_finish(
        &self,
        claim: &CloudClaim,
        outcome: Outcome,
        summary: &str,
    ) -> Result<()> {
        self.transaction(|| {
            self.check_cloud_claim(claim)?;
            let mut task = self.task(&claim.job.work.task_id)?;
            if matches!(
                task.view.status,
                Status::Executing | Status::CancelRequested
            ) {
                task.view.status = if outcome == Outcome::Cancelled {
                    Status::Cancelled
                } else {
                    Status::Failed
                };
                task.view.receipt = Some(Receipt {
                    outcome: outcome.clone(),
                    summary: summary.into(),
                    urls: vec![],
                });
                task.view.execution = Some(ExecutionProgress {
                    message: summary.into(),
                    needs_attention: outcome != Outcome::Cancelled,
                    url: claim
                        .job
                        .session
                        .as_ref()
                        .and_then(|s| crate::devin::owner_session_url(&s.url)),
                });
                Self::clear_lease(&mut task);
                self.save(&mut task, "execution_finished")?;
            }
            self.release_cloud(claim, true)
        })
    }
}
