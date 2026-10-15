use crate::{
    crypto::{Vault, equal, hash, id, secret},
    error::{Error, Result, conflict, require, validate},
    model::*,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

mod channel_actions;
mod cloud;
mod files;
mod identities;
mod permissions;
mod recovery;
pub(crate) use cloud::CloudClaim;

pub const LEASE_TTL: i64 = 120;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs() as i64
}

#[derive(Clone, Serialize, Deserialize)]
struct Task {
    view: TaskView,
    request_hash: String,
    version: u32,
    lease_hash: Option<String>,
    lease_until: Option<i64>,
    lease_credential: Option<String>,
    decision: Option<Decision>,
}

pub struct Store {
    pub(crate) db: Connection,
    pub(crate) vault: Vault,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>, key: [u8; 32]) -> Result<Self> {
        Self::with_clock(path, key, Arc::new(now))
    }

    pub fn with_clock(
        path: impl AsRef<Path>,
        key: [u8; 32],
        clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Result<Self> {
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000; PRAGMA secure_delete = ON;")?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        conflict(
            version <= 3,
            "This database was created by a newer Litecom version.",
        )?;
        db.execute_batch(include_str!("schema.sql"))?;
        let store = Self {
            db,
            vault: Vault::new(key),
            clock,
        };
        let check: Option<Vec<u8>> = store
            .db
            .query_row(
                "SELECT value FROM settings WHERE key = 'key_check'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(check) = check {
            store.vault.decrypt(&check, "key-check")?;
        } else {
            store.db.execute(
                "INSERT INTO settings(key,value) VALUES('key_check',?)",
                [store.vault.encrypt(b"litecom", "key-check")?],
            )?;
        }
        if version < 2 {
            // Earlier alpha builds imposed a seven-day lifetime. Remove future
            // deadlines without restoring access that has already expired.
            store.transaction(|| {
                for mut task in store.task_rows("SELECT id,data FROM tasks", [])? {
                    if task
                        .view
                        .expires_at
                        .is_some_and(|expiry| expiry > store.time())
                    {
                        task.view.expires_at = None;
                        store.db.execute(
                            "UPDATE tasks SET expires_at=?,data=? WHERE id=?",
                            params![
                                i64::MAX,
                                store.vault.seal(&task, &format!("task:{}", task.view.id))?,
                                task.view.id
                            ],
                        )?;
                    }
                }
                store.db.pragma_update(None, "user_version", 2)?;
                Ok(())
            })?;
        }
        if version < 3 {
            // SQLite cannot remove NOT NULL in place. Rebuild only the parent
            // table with FK enforcement temporarily off, then verify all links.
            store.db.pragma_update(None, "foreign_keys", false)?;
            let result = store.transaction(|| {
                let has_kind = store.db.prepare("PRAGMA table_info(channel_challenges)")?.query_map([], |r| r.get::<_, String>(1))?.collect::<std::result::Result<Vec<_>, _>>()?.iter().any(|name| name == "kind");
                if !has_kind { store.db.execute_batch("ALTER TABLE channel_challenges ADD COLUMN kind TEXT NOT NULL DEFAULT 'email';")?; }
                store.db.execute_batch(
                    "CREATE TABLE profiles_v3 (
                        id TEXT PRIMARY KEY, name TEXT NOT NULL, phone_key TEXT UNIQUE,
                        phone BLOB, invite_hash TEXT NOT NULL UNIQUE, invite BLOB NOT NULL
                    );
                    INSERT INTO profiles_v3 SELECT * FROM profiles;
                    DROP TABLE profiles;
                    ALTER TABLE profiles_v3 RENAME TO profiles;",
                )?;
                let invalid: bool = store.db.prepare("PRAGMA foreign_key_check")?.exists([])?;
                conflict(
                    !invalid,
                    "Account migration could not preserve existing relationships.",
                )?;
                store.db.pragma_update(None, "user_version", 3)?;
                Ok(())
            });
            store.db.pragma_update(None, "foreign_keys", true)?;
            result?;
        }
        // Older builds abandoned delivery after eight attempts. Retry active
        // destinations, while preserving the user's choice to disconnect one.
        store.db.execute("UPDATE deliveries SET state=CASE WHEN EXISTS(SELECT 1 FROM channels c WHERE c.id=deliveries.channel_id AND c.verified=1) THEN 'pending' ELSE 'cancelled' END WHERE state='failed'",[])?;
        Ok(store)
    }

    pub(crate) fn time(&self) -> i64 {
        (self.clock)()
    }
    pub(crate) fn transaction<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let result = f()?;
        tx.commit()?;
        Ok(result)
    }

    /// Authentication is the only production caller; the number must have been verified.
    pub(crate) fn verified_profile(&self, name: &str, phone: &str) -> Result<Profile> {
        validate(!name.trim().is_empty(), "Enter your name.")?;
        validate(
            phone.starts_with('+')
                && (2..=16).contains(&phone.len())
                && phone[1..].bytes().all(|b| b.is_ascii_digit())
                && !phone.starts_with("+0"),
            "Enter your phone number with country code, such as +14155550123.",
        )?;
        let fingerprint = self.vault.fingerprint("phone", phone.as_bytes());
        if let Some(user_id) = self
            .db
            .query_row(
                "SELECT id FROM profiles WHERE phone_key=?",
                [&fingerprint],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return self.profile_by_id(&user_id);
        }
        self.transaction(|| {
            let profile = Profile { id: id(), name: name.trim().into(), invite: secret() };
            self.db.execute("INSERT INTO profiles(id,name,phone_key,phone,invite_hash,invite) VALUES(?,?,?,?,?,?)", params![profile.id, profile.name, fingerprint, self.vault.seal(&phone, &format!("phone:{}",profile.id))?, hash(&profile.invite), self.vault.seal(&profile.invite, &format!("invite:{}",profile.id))?])?;
            let agent = Agent { id: id(), owner_id: profile.id.clone(), name: "This computer".into(), kind: AgentKind::Local, is_default: true, revoked: false, last_seen_at: None, connected: false, local: None };
            self.insert_agent(&agent)?;
            Ok(profile)
        })
    }

    pub fn profile(&self, p: &Principal) -> Result<Profile> {
        self.profile_by_id(&p.user_id)
    }
    fn profile_by_id(&self, user_id: &str) -> Result<Profile> {
        let (name, invite): (String, Vec<u8>) = self
            .db
            .query_row(
                "SELECT name,invite FROM profiles WHERE id=?",
                [user_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        Ok(Profile {
            id: user_id.into(),
            name,
            invite: self.vault.open(&invite, &format!("invite:{user_id}"))?,
        })
    }
    pub(crate) fn phone(&self, user_id: &str) -> Result<String> {
        let data: Option<Vec<u8>> =
            self.db
                .query_row("SELECT phone FROM profiles WHERE id=?", [user_id], |r| {
                    r.get(0)
                })?;
        let data = data.ok_or_else(|| {
            Error::Conflict("Connect a phone number before enabling text approvals.".into())
        })?;
        self.vault.open(&data, &format!("phone:{user_id}"))
    }
    pub fn invite_profile(&self, invite: &str) -> Result<(String, String)> {
        self.db
            .query_row(
                "SELECT id,name FROM profiles WHERE invite_hash=?",
                [hash(invite)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)
    }
    pub fn rotate_invite(&self, p: &Principal) -> Result<Profile> {
        p.human()?;
        let invite = secret();
        self.db.execute(
            "UPDATE profiles SET invite_hash=?,invite=? WHERE id=?",
            params![
                hash(&invite),
                self.vault.seal(&invite, &format!("invite:{}", p.user_id))?,
                p.user_id
            ],
        )?;
        self.profile(p)
    }
    fn insert_agent(&self, agent: &Agent) -> Result<()> {
        self.db.execute(
            "INSERT INTO agents(id,owner_id,name,kind,is_default) VALUES(?,?,?,?,?)",
            params![
                agent.id,
                agent.owner_id,
                agent.name,
                serde_json::to_string(&agent.kind)?,
                agent.is_default
            ],
        )?;
        Ok(())
    }
    fn agent_row(r: &rusqlite::Row) -> rusqlite::Result<Agent> {
        let kind: String = r.get("kind")?;
        Ok(Agent {
            id: r.get("id")?,
            owner_id: r.get("owner_id")?,
            name: r.get("name")?,
            kind: serde_json::from_str(&kind).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            is_default: r.get("is_default")?,
            revoked: r.get("revoked")?,
            last_seen_at: r.get("last_seen_at")?,
            connected: false,
            local: None,
        })
    }
    pub fn register_agent(
        &self,
        p: &Principal,
        name: &str,
        kind: AgentKind,
        make_default: bool,
    ) -> Result<Agent> {
        p.human()?;
        validate(!name.trim().is_empty(), "Give this agent a name.")?;
        self.transaction(|| {
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM agents WHERE owner_id=? AND name=? COLLATE NOCASE)",
                params![p.user_id, name.trim()],
                |r| r.get(0),
            )?;
            conflict(!exists, "Give this agent a different name.")?;
            if make_default {
                self.db.execute(
                    "UPDATE agents SET is_default=0 WHERE owner_id=?",
                    [&p.user_id],
                )?;
            }
            let agent = Agent {
                id: id(),
                owner_id: p.user_id.clone(),
                name: name.trim().into(),
                kind,
                is_default: make_default,
                revoked: false,
                last_seen_at: None,
                connected: false,
                local: None,
            };
            self.insert_agent(&agent)?;
            Ok(agent)
        })
    }
    pub(crate) fn owned_agent(&self, p: &Principal, agent_id: &str) -> Result<Agent> {
        let agent = self
            .db
            .query_row(
                "SELECT * FROM agents WHERE id=? AND owner_id=? AND revoked=0",
                params![agent_id, p.user_id],
                Self::agent_row,
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if p.role == Role::Agent {
            require(
                p.agent_id.as_deref() == Some(agent_id),
                "This connection belongs to a different agent.",
            )?;
        }
        Ok(agent)
    }
    pub fn set_default(&self, p: &Principal, agent_id: &str) -> Result<()> {
        p.human()?;
        self.owned_agent(p, agent_id)?;
        self.transaction(|| {
            self.db.execute(
                "UPDATE agents SET is_default=0 WHERE owner_id=?",
                [&p.user_id],
            )?;
            self.db
                .execute("UPDATE agents SET is_default=1 WHERE id=?", [agent_id])?;
            Ok(())
        })
    }
    pub fn agents(&self, p: &Principal, owner_id: &str) -> Result<Vec<Agent>> {
        if p.user_id != owner_id {
            require(
                self.requests_allowed(&p.user_id, owner_id)?,
                "Requests between these accounts are blocked.",
            )?;
        }
        self.agent_list(owner_id)
    }
    pub fn revoke_agent(&self, p: &Principal, agent_id: &str) -> Result<()> {
        p.human()?;
        self.owned_agent(p, agent_id)?;
        self.transaction(|| {
            self.db.execute(
                "UPDATE agents SET revoked=1,is_default=0,config=NULL WHERE id=?",
                [agent_id],
            )?;
            self.db.execute(
                "UPDATE credentials SET revoked=1 WHERE agent_id=?",
                [agent_id],
            )?;
            for mut task in
                self.task_rows("SELECT id,data FROM tasks WHERE agent_id=?", [agent_id])?
            {
                self.stop(&mut task, true)?;
            }
            Ok(())
        })
    }

    fn pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
        if a < b { (a, b) } else { (b, a) }
    }
    fn requests_allowed(&self, a: &str, b: &str) -> Result<bool> {
        let (a, b) = Self::pair(a, b);
        Ok(self
            .db
            .query_row(
                "SELECT status!='blocked' FROM contacts WHERE a=? AND b=?",
                params![a, b],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(true))
    }
    pub fn request_contact(&self, p: &Principal, invite: &str) -> Result<Contact> {
        p.human()?;
        let (target, _) = self.invite_profile(invite)?;
        self.request_contact_to(p, &target)
    }
    // Older clients can still follow saved contact links. Workspace members
    // are immediately addressable; these endpoints never create a pending gate.
    pub fn request_contact_to(&self, p: &Principal, target: &str) -> Result<Contact> {
        p.human()?;
        let name = self.profile_by_id(target)?.name;
        validate(target != p.user_id, "This is your own link.")?;
        Ok(Contact {
            id: target.into(),
            name,
            status: "accepted".into(),
            incoming: false,
            agents: self.agents(p, target)?,
        })
    }
    /// Workspace members can request any listed agent. Private destinations,
    /// credentials, and blocked accounts stay out of the directory.
    pub fn people(&self, p: &Principal) -> Result<Vec<Contact>> {
        p.human()?;
        self.contacts(p)
    }
    pub fn accept_contact(&self, p: &Principal, other: &str) -> Result<()> {
        self.request_contact_to(p, other)?;
        Ok(())
    }
    pub fn block_contact(&self, p: &Principal, other: &str) -> Result<()> {
        p.human()?;
        validate(other != p.user_id, "You cannot block yourself.")?;
        self.profile_by_id(other)?;
        let (a, b) = Self::pair(&p.user_id, other);
        self.transaction(|| {
            self.db.execute("INSERT INTO contacts(a,b,initiator,status,created_at) VALUES(?,?,?,'blocked',?) ON CONFLICT(a,b) DO UPDATE SET status='blocked'",params![a,b,p.user_id,self.time()])?;
            for mut task in self.task_rows("SELECT id,data FROM tasks WHERE (requester_id=? AND owner_id=?) OR (requester_id=? AND owner_id=?)",params![a,b,b,a])? { self.stop(&mut task,true)?; }
            Ok(())
        })
    }
    pub fn contacts(&self, p: &Principal) -> Result<Vec<Contact>> {
        let rows = self
            .db
            .prepare("SELECT id,name FROM profiles WHERE id!=? ORDER BY name COLLATE NOCASE,id")?
            .query_map([&p.user_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut people = Vec::new();
        for (id, name) in rows {
            if !self.requests_allowed(&p.user_id, &id)? {
                continue;
            }
            people.push(Contact {
                agents: self.agent_list(&id)?,
                id,
                name,
                // Preserve the response shape for already installed clients.
                status: "accepted".into(),
                incoming: false,
            });
        }
        Ok(people)
    }
    pub fn resolve(
        &self,
        p: &Principal,
        name: &str,
        agent_name: Option<&str>,
    ) -> Result<(String, String)> {
        if matches!(
            name.trim().to_lowercase().as_str(),
            "me" | "myself" | "self"
        ) || name == p.user_id
        {
            let agents = self.agents(p, &p.user_id)?;
            let agent = agents
                .iter()
                .find(|a| {
                    agent_name
                        .map(|n| a.name.to_lowercase() == n.trim().to_lowercase() || a.id == n)
                        .unwrap_or(a.is_default)
                })
                .ok_or_else(|| Error::Invalid("Choose one of your connected agents.".into()))?;
            return Ok((p.user_id.clone(), agent.id.clone()));
        }
        let contacts = self
            .contacts(p)?
            .into_iter()
            .filter(|c| c.id == name || c.name.to_lowercase() == name.trim().to_lowercase())
            .collect::<Vec<_>>();
        validate(
            !contacts.is_empty(),
            "This person is not available in your workspace.",
        )?;
        conflict(
            contacts.len() == 1,
            "Several people have this name. Select one by ID.",
        )?;
        let contact = &contacts[0];
        let agent = contact
            .agents
            .iter()
            .find(|a| {
                agent_name
                    .map(|n| a.name.to_lowercase() == n.trim().to_lowercase() || a.id == n)
                    .unwrap_or(a.is_default)
            })
            .ok_or_else(|| Error::Invalid("Choose an agent from this person's list.".into()))?;
        Ok((contact.id.clone(), agent.id.clone()))
    }

    fn task(&self, task_id: &str) -> Result<Task> {
        let data: Vec<u8> = self
            .db
            .query_row("SELECT data FROM tasks WHERE id=?", [task_id], |r| r.get(0))
            .optional()?
            .ok_or(Error::NotFound)?;
        self.vault.open(&data, &format!("task:{task_id}"))
    }
    fn task_rows(&self, sql: &str, args: impl rusqlite::Params) -> Result<Vec<Task>> {
        self.db
            .prepare(sql)?
            .query_map(args, |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .map(|r| {
                let (id, data) = r?;
                self.vault.open(&data, &format!("task:{id}"))
            })
            .collect()
    }
    fn save(&self, task: &mut Task, event: &str) -> Result<()> {
        task.view.updated_at = self.time();
        self.db.execute(
            "UPDATE tasks SET status=?,updated_at=?,data=? WHERE id=?",
            params![
                task.view.status.as_str(),
                task.view.updated_at,
                self.vault.seal(task, &format!("task:{}", task.view.id))?,
                task.view.id
            ],
        )?;
        self.db.execute(
            "INSERT INTO events(task_id,kind,created_at) VALUES(?,?,?)",
            params![task.view.id, event, self.time()],
        )?;
        self.enqueue_notifications(self.db.last_insert_rowid())?;
        Ok(())
    }
    fn authorized(&self, p: &Principal, task: &Task) -> Result<()> {
        require(
            task.view.owner_id == p.user_id || task.view.requester_id == p.user_id,
            "This request belongs to another account.",
        )
    }
    fn owner(&self, p: &Principal, task: &Task) -> Result<()> {
        require(
            task.view.owner_id == p.user_id,
            "Only the owner can change this response.",
        )?;
        if p.role == Role::Agent {
            self.owned_agent(p, &task.view.agent_id)?;
        }
        Ok(())
    }
    fn projection(&self, p: &Principal, task: Task) -> Result<TaskView> {
        self.authorized(p, &task)?;
        let mut view = task.view;
        if p.user_id == view.owner_id && view.status == Status::Executing {
            view.permissions = self.pending_permissions(&view.id)?;
        }
        let released = view.status == Status::Completed
            && view.revoked_at.is_none()
            && !view.expired(self.time());
        if released && let Some(draft) = &view.draft {
            view.response = Some(Response {
                text: draft.text.clone(),
                attachments: draft.attachments.clone(),
            });
        }
        if p.user_id != view.owner_id {
            view.draft = None;
            view.execution = None;
            view.permissions.clear();
            if let Some(receipt) = &mut view.receipt {
                receipt.summary = match receipt.outcome {
                    Outcome::Succeeded => "The approved action finished.",
                    Outcome::Failed => "The approved action failed.",
                    Outcome::Cancelled => "The action was cancelled.",
                    Outcome::Uncertain => {
                        "The connection was lost. The owner must check whether the action finished."
                    }
                }
                .into();
                receipt.urls.clear();
            }
        }
        Ok(view)
    }
    pub fn create_task(&self, p: &Principal, input: NewTask) -> Result<TaskView> {
        validate(!input.request.trim().is_empty(), "Describe what you need.")?;
        validate(
            !input.idempotency_key.is_empty(),
            "Provide a request ID for safe retries.",
        )?;
        self.transaction(|| {
            require(p.user_id==input.owner_id || self.requests_allowed(&p.user_id,&input.owner_id)?,"Requests between these accounts are blocked.")?;
            // Bind retries to the original requested selector, not today's default.
            let request_hash=hash(serde_json::to_vec(&(&input.owner_id,&input.agent_id,input.request.trim()))?);
            if let Some(existing)=self.db.query_row("SELECT id FROM tasks WHERE requester_id=? AND idempotency_key=?",params![p.user_id,input.idempotency_key],|r|r.get::<_,String>(0)).optional()? {
                let task=self.task(&existing)?;
                conflict(task.request_hash==request_hash,"This request ID was already used for different work.")?;
                return self.projection(p,task);
            }
            let agent=self.agents(p,&input.owner_id)?.into_iter().find(|a|input.agent_id.as_ref().map(|id|id==&a.id).unwrap_or(a.is_default)).ok_or(Error::NotFound)?;
            let time=self.time();
            let mut task=Task {view:TaskView{id:id(),requester_id:p.user_id.clone(),owner_id:input.owner_id,agent_id:agent.id,status:Status::Queued,request:input.request.trim().into(),created_at:time,updated_at:time,expires_at:None,received_at:None,downloads:vec![],revoked_at:None,draft:None,response:None,receipt:None,error:None,execution:None,permissions:vec![]},request_hash,version:0,lease_hash:None,lease_until:None,lease_credential:None,decision:None};
            self.db.execute("INSERT INTO tasks(id,requester_id,owner_id,agent_id,idempotency_key,status,created_at,updated_at,expires_at,data) VALUES(?,?,?,?,?,?,?,?,?,?)",params![task.view.id,task.view.requester_id,task.view.owner_id,task.view.agent_id,input.idempotency_key,task.view.status.as_str(),time,time,task.view.expires_at.unwrap_or(i64::MAX),self.vault.seal(&task,&format!("task:{}",task.view.id))?])?;
            self.save(&mut task,"queued")?;
            self.projection(p,task)
        })
    }
    pub fn lookup_request(&self, p: &Principal, key: &str) -> Result<Option<TaskView>> {
        let id = self
            .db
            .query_row(
                "SELECT id FROM tasks WHERE requester_id=? AND idempotency_key=?",
                params![p.user_id, key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        id.map(|id| self.get_task(p, &id)).transpose()
    }
    pub fn get_task(&self, p: &Principal, task_id: &str) -> Result<TaskView> {
        self.recover()?;
        self.projection(p, self.task(task_id)?)
    }
    pub fn tasks(&self, p: &Principal, before: Option<(i64, String)>) -> Result<Vec<TaskView>> {
        self.recover()?;
        let (time, cursor) = before.unwrap_or((i64::MAX, String::new()));
        self.task_rows("SELECT id,data FROM tasks WHERE (requester_id=? OR owner_id=?) AND (created_at<? OR (created_at=? AND id<?)) ORDER BY created_at DESC,id DESC LIMIT 100",params![p.user_id,p.user_id,time,time,cursor])?.into_iter().map(|t|self.projection(p,t)).collect()
    }
    fn clear_lease(task: &mut Task) {
        task.lease_hash = None;
        task.lease_until = None;
        task.lease_credential = None;
    }
    pub fn recover(&self) -> Result<()> {
        self.transaction(|| {
            for mut task in self.task_rows("SELECT id,data FROM tasks WHERE status NOT IN ('completed','declined','cancelled','failed','expired')",[])? {
                let expired=task.view.expired(self.time());
                let lost=task.lease_until.is_some_and(|t|t<=self.time());
                let cloud: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM cloud_jobs WHERE task_id=? AND phase IN ('ready','starting','running'))", [&task.view.id], |r|r.get(0))?;
                // The relay can resume monitoring a persisted cloud session. It must
                // never turn a known remote run into a new runnable queue item.
                if cloud && matches!(task.view.status,Status::Executing|Status::CancelRequested) {
                    if expired && task.view.status != Status::CancelRequested {
                        task.view.status=Status::CancelRequested;
                        self.save(&mut task,"cancelled")?;
                    }
                    continue;
                }
                if matches!(task.view.status,Status::Executing|Status::CancelRequested) && (lost||expired) {
                    task.view.status=Status::Failed;
                    task.view.receipt=Some(Receipt{outcome:Outcome::Uncertain,summary:"The runner stopped responding. Check the external result before retrying; Litecom will not repeat the action automatically.".into(),urls:vec![]});
                    task.view.error=Some("Execution outcome unknown.".into());
                } else if expired { task.view.status=Status::Expired; }
                else if lost && task.view.status==Status::Preparing { task.view.status=Status::Queued; }
                else {continue;}
                Self::clear_lease(&mut task);
                self.save(&mut task,"recovered")?;
            }
            Ok(())
        })
    }
    pub fn claim(&self, p: &Principal, agent_id: &str) -> Result<Option<WorkItem>> {
        require(
            self.owned_agent(p, agent_id)?.kind == AgentKind::Local,
            "This cloud agent is run by the relay.",
        )?;
        self.claim_internal(p, agent_id)
    }
    fn claim_internal(&self, p: &Principal, agent_id: &str) -> Result<Option<WorkItem>> {
        require(
            p.role == Role::Agent,
            "Use an agent connection to run work.",
        )?;
        self.owned_agent(p, agent_id)?;
        self.recover()?;
        self.transaction(|| {
            self.db.execute("UPDATE agents SET last_seen_at=? WHERE id=?",params![self.time(),agent_id])?;
            self.db.execute("UPDATE local_pairings SET last_seen_at=? WHERE credential_id=?",params![self.time(),p.credential_id])?;
            let selected=self.db.query_row("SELECT id FROM tasks WHERE agent_id=? AND status IN ('queued','approved') ORDER BY CASE status WHEN 'approved' THEN 0 ELSE 1 END,created_at,id LIMIT 1",[agent_id],|r|r.get::<_,String>(0)).optional()?;
            let Some(task_id)=selected else{return Ok(None)};
            let mut task=self.task(&task_id)?;
            let phase=if task.view.status==Status::Approved {"execute"} else {"prepare"};
            task.view.status=if phase=="execute" {Status::Executing} else {Status::Preparing};
            self.db.execute("UPDATE tool_permissions SET decision='expired' WHERE task_id=? AND decision IS NULL",[&task_id])?;
            let lease=secret();
            task.lease_hash=Some(hash(&lease));task.lease_until=Some(self.time()+LEASE_TTL);task.lease_credential=Some(p.credential_id.clone());
            self.save(&mut task,"claimed")?;
            Ok(Some(WorkItem{task_id,agent_id:agent_id.into(),phase:phase.into(),request:task.view.request,lease,lease_until:task.lease_until.unwrap(),draft:task.view.draft}))
        })
    }
    fn lease(&self, p: &Principal, task: &Task, lease: &str) -> Result<()> {
        self.owner(p, task)?;
        require(
            p.role == Role::Agent
                && task.lease_credential.as_deref() == Some(&p.credential_id)
                && task
                    .lease_hash
                    .as_ref()
                    .is_some_and(|h| equal(h, &hash(lease))),
            "This work lease belongs to another connection.",
        )?;
        conflict(
            task.lease_until.is_some_and(|t| t > self.time()) && !task.view.expired(self.time()),
            "This work lease expired. Check the request status.",
        )
    }
    pub fn heartbeat(&self, p: &Principal, task_id: &str, lease: &str) -> Result<Status> {
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            self.lease(p, &task, lease)?;
            task.lease_until = Some(self.time() + LEASE_TTL);
            // Heartbeats do not create notification events.
            self.db.execute(
                "UPDATE tasks SET data=? WHERE id=?",
                params![self.vault.seal(&task, &format!("task:{task_id}"))?, task_id],
            )?;
            self.db.execute(
                "UPDATE agents SET last_seen_at=? WHERE id=?",
                params![self.time(), task.view.agent_id],
            )?;
            Ok(task.view.status)
        })
    }
    fn artifact(&self, artifact_id: &str) -> Result<Artifact> {
        let data: Vec<u8> = self
            .db
            .query_row(
                "SELECT metadata FROM artifacts WHERE id=?",
                [artifact_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        self.vault
            .open(&data, &format!("artifact-meta:{artifact_id}"))
    }
    pub fn prepare(
        &self,
        p: &Principal,
        task_id: &str,
        lease: Option<&str>,
        input: DraftInput,
    ) -> Result<TaskView> {
        self.save_draft(p, task_id, lease, input, false)
    }

    /// Commit the final result under the live execution lease. Proposals explicitly
    /// approved for execution and delivery release it in the same transaction.
    pub fn stage_result(
        &self,
        p: &Principal,
        task_id: &str,
        lease: &str,
        input: DraftInput,
    ) -> Result<TaskView> {
        require(
            p.role == Role::Agent,
            "Only the executing connection can return a result.",
        )?;
        validate(
            input.action.is_none(),
            "An execution result cannot introduce another action.",
        )?;
        self.save_draft(p, task_id, Some(lease), input, true)
    }

    fn save_draft(
        &self,
        p: &Principal,
        task_id: &str,
        lease: Option<&str>,
        input: DraftInput,
        execution_result: bool,
    ) -> Result<TaskView> {
        input.validate()?;
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            self.owner(p, &task)?;
            conflict(!task.view.expired(self.time()), "This request expired.")?;
            if p.role == Role::Agent {
                self.lease(p, &task, lease.unwrap_or(""))?;
                conflict(
                    task.view.status
                        == if execution_result {
                            Status::Executing
                        } else {
                            Status::Preparing
                        },
                    "This request is no longer accepting this work.",
                )?;
            } else {
                conflict(
                    matches!(
                        task.view.status,
                        Status::Queued | Status::Preparing | Status::AwaitingApproval
                    ),
                    "This response is already decided.",
                )?;
            }
            if execution_result {
                conflict(
                    self.pending_permissions(task_id)?.is_empty(),
                    "Resolve the pending tool permissions before returning a result.",
                )?;
            }
            let share_result = execution_result
                && task.decision == Some(Decision::Approve)
                && task
                    .view
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.action.as_ref())
                    .is_some_and(Action::shares_result);
            let mut attachments = vec![];
            for id in &input.attachment_ids {
                let artifact = self.artifact(id)?;
                require(
                    artifact.task_id == task_id,
                    "This file belongs to another request.",
                )?;
                validate(
                    !attachments.iter().any(|a: &Artifact| a.id == *id),
                    "Remove duplicate attachments.",
                )?;
                attachments.push(artifact);
            }
            task.version += 1;
            let digest = hash(serde_json::to_vec(&(
                &task.view.id,
                &task.view.requester_id,
                &task.view.owner_id,
                &task.view.agent_id,
                task.version,
                &input.text,
                &attachments,
                &input.action,
            ))?);
            task.view.draft = Some(Draft {
                text: input.text,
                attachments,
                action: input.action,
                version: task.version,
                digest,
            });
            task.view.status = if share_result {
                Status::Completed
            } else {
                Status::AwaitingApproval
            };
            if execution_result {
                task.view.receipt = Some(Receipt {
                    outcome: Outcome::Succeeded,
                    summary: if share_result {
                        "The agent finished and sent the result."
                    } else {
                        "The agent finished. Review the reply and files before sharing them."
                    }
                    .into(),
                    urls: vec![],
                });
                if share_result && let Some(progress) = task.view.execution.as_mut() {
                    progress.message = "The agent finished and sent the result.".into();
                    progress.needs_attention = false;
                }
            }
            task.decision = if share_result {
                Some(Decision::Approve)
            } else {
                None
            };
            Self::clear_lease(&mut task);
            self.save(
                &mut task,
                if share_result {
                    "completed"
                } else {
                    "awaiting_approval"
                },
            )?;
            self.projection(p, task)
        })
    }
    pub fn decide(
        &self,
        p: &Principal,
        task_id: &str,
        version: u32,
        digest: &str,
        decision: Decision,
    ) -> Result<TaskView> {
        p.human()?;
        self.transaction(|| {
            let mut task=self.task(task_id)?;self.owner(p,&task)?;
            conflict(!task.view.expired(self.time()),"This request expired.")?;
            let draft=task.view.draft.as_ref().ok_or_else(||Error::Conflict("The reply is not ready to review.".into()))?;
            conflict(version==draft.version && equal(digest,&draft.digest),"This reply changed. Review the latest version.")?;
            if let Some(existing)=&task.decision {conflict(*existing==decision,"This request was already decided.")?;return self.projection(p,task);}
            conflict(task.view.status==Status::AwaitingApproval,"This request is no longer waiting for approval.")?;
            task.view.status=match decision {Decision::Decline=>Status::Declined,Decision::Approve if draft.action.is_some()=>Status::Approved,Decision::Approve=>Status::Completed};
            if task.view.status==Status::Completed {
                if let Some(receipt)=task.view.receipt.as_mut() {
                    receipt.summary="The reply and files were approved for sharing.".into();
                }
                if let Some(progress)=task.view.execution.as_mut() {
                    progress.message="The reply and files were approved for sharing.".into();
                    progress.needs_attention=false;
                }
            }
            task.decision=Some(decision.clone());
            self.db.execute("INSERT INTO approvals(task_id,version,digest,user_id,decision,created_at) VALUES(?,?,?,?,?,?)",params![task_id,version,digest,p.user_id,serde_json::to_string(&decision)?,self.time()])?;
            self.save(&mut task,if decision==Decision::Approve {"approved"} else {"declined"})?;
            self.projection(p,task)
        })
    }
    pub fn finish(
        &self,
        p: &Principal,
        task_id: &str,
        lease: &str,
        receipt: Receipt,
    ) -> Result<TaskView> {
        validate(
            receipt.urls.iter().all(|u| {
                url::Url::parse(u).is_ok_and(|u| {
                    u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
                })
            }),
            "Provide a short result with HTTPS links.",
        )?;
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            self.lease(p, &task, lease)?;
            conflict(
                matches!(
                    task.view.status,
                    Status::Executing | Status::CancelRequested
                ),
                "This request is not executing.",
            )?;
            task.view.status = match receipt.outcome {
                Outcome::Succeeded => Status::Completed,
                Outcome::Cancelled => Status::Cancelled,
                _ => Status::Failed,
            };
            task.view.receipt = Some(receipt);
            Self::clear_lease(&mut task);
            self.save(&mut task, "execution_finished")?;
            self.projection(p, task)
        })
    }
    fn stop(&self, task: &mut Task, revoke: bool) -> Result<()> {
        if revoke && task.view.revoked_at.is_none() {
            task.view.revoked_at = Some(self.time());
        }
        if matches!(
            task.view.status,
            Status::Executing | Status::CancelRequested
        ) {
            task.view.status = Status::CancelRequested;
        } else if !task.view.status.terminal() {
            task.view.status = Status::Cancelled;
            Self::clear_lease(task);
        }
        self.save(task, "cancelled")
    }
    pub fn cancel(&self, p: &Principal, task_id: &str) -> Result<TaskView> {
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            self.authorized(p, &task)?;
            let revoke = p.user_id == task.view.owner_id;
            self.stop(&mut task, revoke)?;
            self.projection(p, task)
        })
    }
    pub fn acknowledge_download(
        &self,
        p: &Principal,
        artifact_id: &str,
        sha256: &str,
    ) -> Result<TaskView> {
        self.transaction(|| {
            let artifact = self.artifact(artifact_id)?;
            let mut task = self.task(&artifact.task_id)?;
            require(
                p.user_id == task.view.requester_id,
                "Only the recipient can report a download.",
            )?;
            require(
                task.view.status == Status::Completed
                    && task.view.revoked_at.is_none()
                    && !task.view.expired(self.time())
                    && task
                        .view
                        .draft
                        .as_ref()
                        .is_some_and(|d| d.attachments.iter().any(|a| a.id == artifact_id)),
                "This file is not available.",
            )?;
            validate(
                equal(&artifact.sha256, sha256),
                "The downloaded file did not match the approved snapshot.",
            )?;
            if !task
                .view
                .downloads
                .iter()
                .any(|d| d.artifact_id == artifact_id)
            {
                task.view.downloads.push(DownloadReceipt {
                    artifact_id: artifact_id.into(),
                    verified_at: self.time(),
                });
                self.save(&mut task, "download_verified")?;
            }
            self.projection(p, task)
        })
    }
    pub fn acknowledge(&self, p: &Principal, task_id: &str) -> Result<TaskView> {
        self.transaction(|| {
            let mut task = self.task(task_id)?;
            require(
                p.user_id == task.view.requester_id,
                "Only the recipient can acknowledge receipt.",
            )?;
            conflict(
                task.view.status == Status::Completed
                    && task.view.revoked_at.is_none()
                    && !task.view.expired(self.time()),
                "This response is not available.",
            )?;
            if task.view.received_at.is_none() {
                task.view.received_at = Some(self.time());
                self.save(&mut task, "received")?;
            }
            self.projection(p, task)
        })
    }
    pub fn delivery_title(&self, delivery: &crate::notifications::Delivery) -> Result<String> {
        // Read current state when sending, including retries. An old event must
        // not tell the user to approve work that is already decided or cancelled.
        let task = self.task(&delivery.task_id)?;
        if task.view.status == Status::AwaitingApproval && !task.view.expired(self.time()) {
            let requester = self.profile_by_id(&task.view.requester_id)?;
            let words = task
                .view
                .request
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let mut preview = words.chars().take(180).collect::<String>();
            if preview.len() < words.len() {
                preview.push('…');
            }
            return Ok(format!(
                "{} asks: “{preview}” Approve or decline:",
                requester.name
            ));
        }
        Ok(if task.view.expired(self.time())
            && !matches!(
                task.view.status,
                Status::Executing | Status::CancelRequested
            )
        {
            "The request expired."
        } else if task.view.revoked_at.is_some() && task.view.status == Status::Completed {
            "Access to this response was revoked."
        } else {
            match task.view.status {
                Status::Queued | Status::Preparing => "A new request is waiting for your agent.",
                Status::AwaitingApproval => "A request is ready for your review.",
                Status::Approved => "The run was approved and is waiting for the agent.",
                Status::Executing if !self.pending_permissions(&delivery.task_id)?.is_empty() => {
                    "An agent is waiting for your approval of a tool operation."
                }
                Status::Executing
                    if task
                        .view
                        .execution
                        .as_ref()
                        .is_some_and(|p| p.needs_attention) =>
                {
                    "Your cloud agent needs attention."
                }
                Status::Executing => "The approved agent is working.",
                Status::CancelRequested => {
                    "Cancellation is waiting for the agent to confirm it stopped."
                }
                Status::Completed if delivery.kind == "download_verified" => {
                    "A file download was verified by the recipient."
                }
                Status::Completed if task.view.received_at.is_some() => {
                    "The response was received."
                }
                Status::Completed => "The response was approved and is available.",
                Status::Declined => "The request was declined.",
                Status::Cancelled => "The request was cancelled.",
                Status::Failed => "A request needs your attention.",
                Status::Expired => "The request expired.",
            }
        }
        .to_owned())
    }
    pub fn events(&self, p: &Principal, after: i64) -> Result<Vec<Event>> {
        Ok(self.db.prepare("SELECT e.sequence,e.task_id,e.kind,e.created_at FROM events e JOIN tasks t ON t.id=e.task_id WHERE e.sequence>? AND (t.requester_id=? OR t.owner_id=?) ORDER BY e.sequence LIMIT 100")?.query_map(params![after,p.user_id,p.user_id],|r|Ok(Event{sequence:r.get(0)?,task_id:r.get(1)?,kind:r.get(2)?,created_at:r.get(3)?}))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
}
