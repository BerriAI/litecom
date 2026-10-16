use super::*;

/// Host resource limits. These do not change an approved agent's tool permissions.
#[derive(Clone, Debug)]
pub struct Limits {
    pub requests_per_minute: u64,
    pub pending_requests: u64,
    pub slack_per_minute: u64,
    pub pending_slack_jobs: u64,
    pub file_bytes: u64,
    pub task_file_bytes: u64,
    pub stored_file_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            pending_requests: 200,
            slack_per_minute: 30,
            pending_slack_jobs: 1000,
            file_bytes: 1024 * 1024 * 1024,
            task_file_bytes: 5 * 1024 * 1024 * 1024,
            stored_file_bytes: 50 * 1024 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub fn from_env() -> Result<Self> {
        let mut limits = Self::default();
        for (name, value) in [
            (
                "LITECOM_REQUESTS_PER_MINUTE",
                &mut limits.requests_per_minute,
            ),
            ("LITECOM_PENDING_REQUESTS", &mut limits.pending_requests),
            ("LITECOM_SLACK_PER_MINUTE", &mut limits.slack_per_minute),
            ("LITECOM_PENDING_SLACK_JOBS", &mut limits.pending_slack_jobs),
            ("LITECOM_MAX_FILE_BYTES", &mut limits.file_bytes),
            ("LITECOM_MAX_TASK_FILE_BYTES", &mut limits.task_file_bytes),
            (
                "LITECOM_MAX_STORED_FILE_BYTES",
                &mut limits.stored_file_bytes,
            ),
        ] {
            if let Some(raw) = std::env::var_os(name) {
                *value = raw
                    .to_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "{name} must be a positive integer no larger than {}.",
                            i64::MAX
                        ))
                    })?;
            }
        }
        Ok(limits)
    }
}

impl Store {
    pub(crate) fn check_workspace(&self, user: &str) -> Result<()> {
        if let Some(google) = &self.workspace_policy {
            require(
                match self.google_identity_for(user)? {
                    Some(identity) => google.permits(&identity),
                    None => !google.restricts_workspace(),
                },
                "Sign in with a Google account from this server's approved workspace.",
            )?;
        }
        Ok(())
    }

    pub(super) fn task_workspace_allowed(&self, task: &Task) -> Result<bool> {
        for user in [&task.view.owner_id, &task.view.requester_id] {
            match self.check_workspace(user) {
                Ok(()) => {}
                Err(Error::Forbidden(_)) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    pub(super) fn check_request_capacity(&self, user: &str) -> Result<()> {
        let recent: u64 = self.db.query_row(
            "SELECT COUNT(*) FROM tasks WHERE requester_id=? AND created_at>?",
            params![user, self.time() - 60],
            |r| r.get(0),
        )?;
        if recent >= self.limits.requests_per_minute {
            return Err(Error::RateLimited);
        }
        let pending: u64 = self.db.query_row(
            "SELECT COUNT(*) FROM tasks WHERE requester_id=? AND status IN ('queued','preparing','awaiting_approval','approved','executing','cancel_requested')",
            [user], |r| r.get(0))?;
        if pending >= self.limits.pending_requests {
            return Err(Error::Conflict("You have too many unfinished requests. Finish or cancel one before sending another.".into()));
        }
        Ok(())
    }

    // Called within the same transaction as enqueueing. Retries of an existing
    // event are checked first and never consume another slot.
    pub(super) fn slack_capacity(&self, actor: &str) -> Result<()> {
        let now = self.time();
        self.db
            .execute("DELETE FROM intake_limits WHERE expires_at<=?", [now])?;
        let pending: u64 =
            self.db
                .query_row("SELECT COUNT(*) FROM slack_jobs WHERE done=0", [], |r| {
                    r.get(0)
                })?;
        let recent: u64 = self.db.query_row(
            "SELECT COALESCE((SELECT count FROM intake_limits WHERE actor=?),0)",
            [actor],
            |r| r.get(0),
        )?;
        if pending >= self.limits.pending_slack_jobs || recent >= self.limits.slack_per_minute {
            return Err(Error::RateLimited);
        }
        self.db.execute("INSERT INTO intake_limits(actor,count,expires_at) VALUES(?,1,?) ON CONFLICT(actor) DO UPDATE SET count=count+1", params![actor,now+60])?;
        Ok(())
    }
}
