//! A one-time permission is bound to one concrete tool call and live work lease.
use super::*;

#[derive(Serialize, Deserialize)]
struct Record {
    permission: ToolPermission,
    credential_id: String,
    lease_hash: String,
}
impl Store {
    fn permission_record(&self, task_id: &str, id: &str) -> Result<Record> {
        let data: Vec<u8> = self
            .db
            .query_row(
                "SELECT data FROM tool_permissions WHERE task_id=? AND id=?",
                params![task_id, id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        self.vault
            .open(&data, &format!("tool-permission:{task_id}:{id}"))
    }
    fn save_permission(&self, task_id: &str, record: &Record) -> Result<()> {
        let p = &record.permission;
        let decision = p.decision.as_ref().map(serde_json::to_string).transpose()?;
        self.db.execute("INSERT INTO tool_permissions(task_id,id,decision,data) VALUES(?,?,?,?) ON CONFLICT(task_id,id) DO UPDATE SET decision=excluded.decision,data=excluded.data",params![task_id,p.id,decision,self.vault.seal(record,&format!("tool-permission:{task_id}:{}",p.id))?])?;
        Ok(())
    }
    pub(crate) fn pending_permissions(&self, task_id: &str) -> Result<Vec<ToolPermission>> {
        let ids=self.db.prepare("SELECT id FROM tool_permissions WHERE task_id=? AND decision IS NULL ORDER BY rowid")?.query_map([task_id],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        ids.into_iter()
            .map(|id| self.permission_record(task_id, &id).map(|r| r.permission))
            .collect()
    }
    pub fn request_permission(
        &self,
        p: &Principal,
        task_id: &str,
        lease: &str,
        input: ToolPermissionInput,
    ) -> Result<ToolPermission> {
        validate(
            uuid::Uuid::parse_str(&input.id).is_ok(),
            "Provide an ID for this tool operation.",
        )?;
        validate(
            !input.title.trim().is_empty()
                && input.title.len() <= 200
                && serde_json::to_vec(&input.details)?.len() <= 100_000,
            "Describe the tool operation and its scope.",
        )?;
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            self.lease(p, &task, lease)?;
            conflict(
                task.view.status == Status::Executing,
                "This request is no longer executing.",
            )?;
            let digest = hash(serde_json::to_vec(&(
                task_id,
                &p.credential_id,
                hash(lease),
                &input,
            ))?);
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tool_permissions WHERE task_id=? AND id=?)",
                params![task_id, input.id],
                |r| r.get(0),
            )?;
            if exists {
                let old = self.permission_record(task_id, &input.id)?;
                conflict(
                    equal(&old.permission.digest, &digest),
                    "This permission ID already describes a different operation.",
                )?;
                return Ok(old.permission);
            }
            let count: i64 = self.db.query_row(
                "SELECT COUNT(*) FROM tool_permissions WHERE task_id=? AND decision IS NULL",
                [task_id],
                |r| r.get(0),
            )?;
            validate(
                count < 100,
                "Wait for the pending tool decisions before requesting more.",
            )?;
            let record = Record {
                permission: ToolPermission {
                    id: input.id,
                    title: input.title,
                    details: input.details,
                    digest,
                    decision: None,
                    created_at: self.time(),
                },
                credential_id: p.credential_id.clone(),
                lease_hash: hash(lease),
            };
            self.save_permission(task_id, &record)?;
            self.save(&mut task, "permission_requested")?;
            Ok(record.permission)
        })
    }
    pub fn permission_status(
        &self,
        p: &Principal,
        task_id: &str,
        lease: &str,
        id: &str,
    ) -> Result<ToolPermission> {
        let task = self.task(task_id)?;
        self.lease(p, &task, lease)?;
        conflict(
            task.view.status == Status::Executing,
            "This request is no longer executing.",
        )?;
        let record = self.permission_record(task_id, id)?;
        require(
            record.credential_id == p.credential_id && equal(&record.lease_hash, &hash(lease)),
            "This tool permission belongs to another run.",
        )?;
        Ok(record.permission)
    }
    pub fn decide_permission(
        &self,
        p: &Principal,
        task_id: &str,
        id: &str,
        digest: &str,
        decision: Decision,
    ) -> Result<ToolPermission> {
        p.human()?;
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            self.owner(p, &task)?;
            conflict(
                task.view.status == Status::Executing
                    && task.view.expires_at > self.time()
                    && task.lease_until.is_some_and(|until| until > self.time()),
                "The agent is no longer waiting on this run. Check the request status.",
            )?;
            let mut record = self.permission_record(task_id, id)?;
            conflict(
                task.lease_hash.as_deref() == Some(&record.lease_hash)
                    && task.lease_credential.as_deref() == Some(&record.credential_id),
                "This tool permission belongs to a previous run.",
            )?;
            conflict(
                equal(&record.permission.digest, digest),
                "This tool operation changed. Review the current request.",
            )?;
            if let Some(old) = &record.permission.decision {
                conflict(*old == decision, "This tool operation was already decided.")?;
                return Ok(record.permission);
            }
            record.permission.decision = Some(decision);
            self.save_permission(task_id, &record)?;
            self.save(&mut task, "permission_decided")?;
            Ok(record.permission)
        })
    }
}
