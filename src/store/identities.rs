use super::*;
use crate::google::Identity;

impl Store {
    pub(crate) fn agent_list(&self, owner_id: &str) -> Result<Vec<Agent>> {
        let agents = self
            .db
            .prepare(
                "SELECT * FROM agents WHERE owner_id=? AND revoked=0 ORDER BY is_default DESC,name",
            )?
            .query_map([owner_id], Self::agent_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        agents.into_iter().map(|mut agent| {
            let config: Option<Vec<u8>> = self.db.query_row("SELECT config FROM agents WHERE id=?", [&agent.id], |r| r.get(0))?;
            agent.connected = if agent.kind == AgentKind::Local {
                agent.local = config.map(|data| self.vault.open(&data, &format!("agent-config:{}", agent.id))).transpose()?;
                agent.last_seen_at.is_some() && self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM credentials WHERE agent_id=? AND role='agent' AND revoked=0 AND expires_at>?)",
                    params![agent.id, self.time()], |r| r.get::<_,bool>(0),
                )?
            } else { config.is_some() };
            Ok(agent)
        }).collect()
    }

    /// A worker can identify only the local agent bound to its credential.
    pub(crate) fn identify_local_agent(
        &self,
        p: &Principal,
        identity: &LocalAgentIdentity,
    ) -> Result<()> {
        require(
            p.role == Role::Agent,
            "Use this computer's agent connection.",
        )?;
        let agent = self.owned_agent(p, p.agent_id.as_deref().ok_or(Error::Unauthorized)?)?;
        require(agent.kind == AgentKind::Local, "This is not a local agent.")?;
        for value in [&identity.device_name, &identity.harness] {
            validate(
                !value.trim().is_empty() && !value.chars().any(char::is_control),
                "Provide the computer and agent names.",
            )?;
        }
        self.transaction(|| {
            if matches!(agent.name.as_str(), "Default" | "This computer") {
                let mut name = identity.device_name.clone();
                let mut suffix = 2;
                while self.db.query_row("SELECT EXISTS(SELECT 1 FROM agents WHERE owner_id=? AND name=? COLLATE NOCASE AND id!=?)", params![p.user_id, name, agent.id], |r| r.get::<_,bool>(0))? {
                    name = format!("{} ({suffix})", identity.device_name);
                    suffix += 1;
                }
                self.db.execute("UPDATE agents SET name=? WHERE id=?", params![name, agent.id])?;
            }
            self.db.execute("UPDATE agents SET config=? WHERE id=?", params![self.vault.seal(identity, &format!("agent-config:{}", agent.id))?, agent.id])?;
            Ok(())
        })
    }

    pub(crate) fn has_phone(&self, p: &Principal) -> Result<bool> {
        p.human()?;
        Ok(self.db.query_row(
            "SELECT phone IS NOT NULL FROM profiles WHERE id=?",
            [&p.user_id],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn save_login_state<T: Serialize>(
        &self,
        state: &str,
        browser: &str,
        data: &T,
    ) -> Result<()> {
        let key = hash(state);
        self.db.execute(
            "DELETE FROM login_states WHERE expires_at<=?",
            [self.time()],
        )?;
        self.db.execute(
            "INSERT INTO login_states(state_hash,browser_hash,data,expires_at) VALUES(?,?,?,?)",
            params![
                key,
                hash(browser),
                self.vault.seal(data, &format!("login:{key}"))?,
                self.time() + 600
            ],
        )?;
        Ok(())
    }

    pub(crate) fn take_login_state<T: serde::de::DeserializeOwned>(
        &self,
        state: &str,
        browser: &str,
    ) -> Result<T> {
        let key = hash(state);
        self.transaction(|| {
            let data: Vec<u8> = self.db.query_row(
                "DELETE FROM login_states WHERE state_hash=? AND browser_hash=? AND expires_at>? RETURNING data",
                params![key, hash(browser), self.time()], |r| r.get(0),
            ).optional()?.ok_or_else(|| Error::Invalid("This sign-in expired or was already used. Start sign-in again in this browser.".into()))?;
            self.vault.open(&data, &format!("login:{key}"))
        })
    }

    /// Called only after Google has authenticated the stable subject. Email is
    /// metadata, never a key for automatically merging accounts.
    pub(crate) fn google_profile(&self, identity: &Identity) -> Result<Profile> {
        self.transaction(|| {
            let key = self.vault.fingerprint("google-sub", identity.subject.as_bytes());
            let existing: Option<String> = self.db.query_row(
                "SELECT user_id FROM login_identities WHERE provider='google' AND subject_key=?",
                [&key], |r| r.get(0),
            ).optional()?;
            let profile = if let Some(user) = existing {
                self.profile_by_id(&user)?
            } else {
                let profile = Profile { id: id(), name: identity.name.clone(), invite: secret() };
                self.db.execute("INSERT INTO profiles(id,name,invite_hash,invite) VALUES(?,?,?,?)",
                    params![profile.id, profile.name, hash(&profile.invite), self.vault.seal(&profile.invite, &format!("invite:{}", profile.id))?])?;
                self.insert_agent(&Agent { id: id(), owner_id: profile.id.clone(), name: "This computer".into(), kind: AgentKind::Local, is_default: true, revoked: false, last_seen_at: None, connected: false, local: None })?;
                profile
            };
            self.db.execute("INSERT INTO login_identities(provider,subject_key,user_id,data) VALUES('google',?,?,?) ON CONFLICT(provider,subject_key) DO UPDATE SET data=excluded.data",
                params![key, profile.id, self.vault.seal(identity, &format!("google:{key}"))?])?;
            Ok(profile)
        })
    }

    pub(crate) fn google_identity(&self, p: &Principal) -> Result<Option<Identity>> {
        p.human()?;
        self.google_identity_for(&p.user_id)
    }
    pub(crate) fn google_identity_for(&self, user_id: &str) -> Result<Option<Identity>> {
        let row: Option<(String,Vec<u8>)> = self.db.query_row(
            "SELECT subject_key,data FROM login_identities WHERE provider='google' AND user_id=?",
            [user_id], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        row.map(|(key, data)| self.vault.open(&data, &format!("google:{key}")))
            .transpose()
    }

    pub(crate) fn connect_google_email(&self, p: &Principal) -> Result<()> {
        let identity = self.google_identity(p)?.ok_or_else(|| {
            Error::Conflict("Sign in with Google or verify another email address.".into())
        })?;
        self.verified_channel(p, "email", &identity.email)
    }
}
