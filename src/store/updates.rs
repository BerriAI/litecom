//! Short, renewable claim pauses also protect workers from older releases.
use super::*;

#[derive(Serialize, Deserialize)]
struct UpdatePause {
    id: String,
    credential: String,
    until: i64,
}
impl Store {
    fn update_pause(&self, agent: &str) -> Result<Option<UpdatePause>> {
        let bytes: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT value FROM settings WHERE key=?",
                [format!("worker-update:{agent}")],
                |r| r.get(0),
            )
            .optional()?;
        bytes.map(|b| Ok(serde_json::from_slice(&b)?)).transpose()
    }
    pub(super) fn worker_updating(&self, agent: &str) -> Result<bool> {
        Ok(self
            .update_pause(agent)?
            .is_some_and(|pause| pause.until > self.time()))
    }
    pub fn worker_update(&self, p: &Principal, id: &str, release: bool) -> Result<usize> {
        require(
            p.role == Role::Agent,
            "Use the installed agent connection to update.",
        )?;
        self.check_workspace(&p.user_id)?;
        let agent = p.agent_id.as_deref().ok_or(Error::Unauthorized)?;
        self.owned_agent(p, agent)?;
        validate(uuid::Uuid::parse_str(id).is_ok(), "Invalid update ID.")?;
        self.recover()?;
        self.transaction(|| {
            let key = format!("worker-update:{agent}");
            if let Some(pause) = self.update_pause(agent)? {
                conflict(pause.until <= self.time() || (pause.id == id && pause.credential == p.credential_id), "Another update is already running for this connection.")?;
            }
            if release {
                self.db.execute("DELETE FROM settings WHERE key=?", [&key])?;
            } else {
                let pause = UpdatePause { id: id.into(), credential: p.credential_id.clone(), until: self.time() + 180 };
                self.db.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_vec(&pause)?])?;
            }
            Ok(self.db.query_row("SELECT COUNT(*) FROM tasks WHERE agent_id=? AND status IN ('preparing','executing','cancel_requested')", [agent], |r| r.get(0))?)
        })
    }
}
