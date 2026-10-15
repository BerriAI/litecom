use crate::{
    crypto::{equal, hash, id, secret},
    error::{Error, Result, conflict, require, validate},
    model::*,
    store::Store,
};
use rand::Rng;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Challenge {
    phone: String,
    name: String,
    browser_hash: String,
}
#[derive(Serialize)]
pub struct OtpChallenge {
    pub id: String,
    pub expires_in: u32,
}
#[derive(Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub csrf: String,
    pub profile: Profile,
}
#[derive(Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: u32,
    pub interval: u32,
}
#[derive(Serialize, Deserialize)]
pub struct AgentToken {
    pub token: String,
    pub user_id: String,
    pub agent_id: String,
    pub expires_at: i64,
}
#[derive(Serialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub label: String,
    pub agent_id: Option<String>,
    pub expires_at: i64,
}

impl Store {
    pub fn begin_login(
        &self,
        name: &str,
        phone: &str,
        browser: &str,
    ) -> Result<(OtpChallenge, String)> {
        validate(!name.trim().is_empty(), "Enter your name.")?;
        validate(
            phone.starts_with('+')
                && !phone.starts_with("+0")
                && (2..=16).contains(&phone.len())
                && phone[1..].bytes().all(|b| b.is_ascii_digit()),
            "Use your phone number with country code.",
        )?;
        self.transaction(|| {
            let key = self.vault.fingerprint("phone", phone.as_bytes());
            let count: i64 = self.db.query_row(
                "SELECT COUNT(*) FROM otp WHERE phone_key=? AND expires_at>?",
                params![key, self.time() - 3000],
                |r| r.get(0),
            )?;
            if count >= 5 {
                return Err(Error::RateLimited);
            }
            let recent: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM otp WHERE phone_key=? AND expires_at>?)",
                params![key, self.time() + 540],
                |r| r.get(0),
            )?;
            if recent {
                return Err(Error::RateLimited);
            }
            let challenge_id = id();
            let code = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000));
            let code_hash = self
                .vault
                .fingerprint(&format!("otp:{challenge_id}"), code.as_bytes());
            let data = Challenge {
                phone: phone.into(),
                name: name.trim().into(),
                browser_hash: hash(browser),
            };
            self.db.execute(
                "INSERT INTO otp(id,phone_key,code_hash,data,expires_at) VALUES(?,?,?,?,?)",
                params![
                    challenge_id,
                    key,
                    code_hash,
                    self.vault.seal(&data, &format!("otp:{challenge_id}"))?,
                    self.time() + 600
                ],
            )?;
            Ok((
                OtpChallenge {
                    id: challenge_id,
                    expires_in: 600,
                },
                code,
            ))
        })
    }
    pub fn verify_login(&self, challenge_id: &str, code: &str, browser: &str) -> Result<Session> {
        // Failed attempts must commit too; an error inside a transaction would undo the counter.
        let checked = self.transaction(|| {
            let row = self
                .db
                .query_row(
                    "SELECT code_hash,data,expires_at,attempts,used FROM otp WHERE id=?",
                    [challenge_id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Vec<u8>>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, bool>(4)?,
                        ))
                    },
                )
                .optional()?;
            let Some((expected, data, expires, attempts, used)) = row else {
                return Ok(None);
            };
            if used || expires <= self.time() || attempts >= 5 {
                return Ok(None);
            };
            let challenge: Challenge = self.vault.open(&data, &format!("otp:{challenge_id}"))?;
            let supplied = self
                .vault
                .fingerprint(&format!("otp:{challenge_id}"), code.as_bytes());
            let good =
                equal(&expected, &supplied) && equal(&challenge.browser_hash, &hash(browser));
            self.db.execute(
                "UPDATE otp SET attempts=attempts+1,used=? WHERE id=?",
                params![good, challenge_id],
            )?;
            Ok(if good { Some(challenge) } else { None })
        })?;
        let challenge = checked.ok_or_else(|| {
            Error::Invalid(
                "That code is incorrect or expired. Request a new code if needed.".into(),
            )
        })?;
        let profile = self.verified_profile(&challenge.name, &challenge.phone)?;
        let token = secret();
        let csrf = secret();
        self.db.execute("INSERT INTO credentials(id,user_id,token_hash,role,label,csrf_hash,expires_at) VALUES(?,?,?,'human','Browser',?,?)",params![id(),profile.id,hash(&token),hash(&csrf),self.time()+30*24*60*60])?;
        Ok(Session {
            token,
            csrf,
            profile,
        })
    }
    pub fn authenticate(&self, token: &str, role: Role) -> Result<Principal> {
        let role_str = if role == Role::Human {
            "human"
        } else {
            "agent"
        };
        let result=self.db.query_row("SELECT id,user_id,agent_id FROM credentials WHERE token_hash=? AND role=? AND revoked=0 AND expires_at>?",params![hash(token),role_str,self.time()],|r|Ok(Principal{credential_id:r.get(0)?,user_id:r.get(1)?,agent_id:r.get(2)?,role:role.clone()})).optional()?.ok_or(Error::Unauthorized)?;
        if let Some(agent) = &result.agent_id {
            let active: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM agents WHERE id=? AND revoked=0)",
                [agent],
                |r| r.get(0),
            )?;
            if !active {
                return Err(Error::Unauthorized);
            }
        }
        Ok(result)
    }
    pub fn check_csrf(&self, p: &Principal, csrf: &str) -> Result<()> {
        p.human()?;
        let expected: Option<String> = self.db.query_row(
            "SELECT csrf_hash FROM credentials WHERE id=?",
            [&p.credential_id],
            |r| r.get(0),
        )?;
        require(
            expected.is_some_and(|v| equal(&v, &hash(csrf))),
            "Reload the page before trying again.",
        )
    }
    pub fn begin_device(&self, label: &str) -> Result<DeviceCode> {
        validate(!label.trim().is_empty(), "Name this connection.")?;
        let device_code = secret();
        // Eight base32 characters: 40 bits. Device requests are additionally rate limited by the relay.
        let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let user_code = (0..8)
            .map(|_| alphabet[rand::thread_rng().gen_range(0..alphabet.len())] as char)
            .collect::<String>();
        self.db.execute(
            "INSERT INTO devices(device_hash,user_code,label,expires_at) VALUES(?,?,?,?)",
            params![
                hash(&device_code),
                user_code,
                label.trim(),
                self.time() + 600
            ],
        )?;
        Ok(DeviceCode {
            device_code,
            user_code,
            expires_in: 600,
            interval: 3,
        })
    }
    pub fn device_label(&self, user_code: &str) -> Result<String> {
        self.db
            .query_row(
                "SELECT label FROM devices WHERE user_code=? AND expires_at>? AND consumed=0",
                params![user_code, self.time()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)
    }
    pub fn authorize_device(&self, p: &Principal, user_code: &str, agent_id: &str) -> Result<()> {
        p.human()?;
        require(
            self.agents(p, &p.user_id)?.iter().any(|a| a.id == agent_id),
            "Choose one of your agents.",
        )?;
        self.transaction(|| {
            let count=self.db.execute("UPDATE devices SET user_id=?,agent_id=? WHERE user_code=? AND expires_at>? AND consumed=0 AND user_id IS NULL",params![p.user_id,agent_id,user_code,self.time()])?;
            conflict(count==1,"This connection code expired or was already used.")
        })
    }
    pub fn poll_device(&self, device_code: &str) -> Result<Option<AgentToken>> {
        self.transaction(|| {
            let (label,user,agent,expires,consumed):(String,Option<String>,Option<String>,i64,bool)=self.db.query_row("SELECT label,user_id,agent_id,expires_at,consumed FROM devices WHERE device_hash=?",[hash(device_code)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or(Error::NotFound)?;
            conflict(expires>self.time() && !consumed,"This connection code expired or was already used.")?;
            let (Some(user_id),Some(agent_id))=(user,agent) else{return Ok(None)};
            let active:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM agents WHERE id=? AND revoked=0)",[&agent_id],|r|r.get(0))?;
            conflict(active,"This agent was disconnected.")?;
            let token=secret();let expires_at=self.time()+365*24*60*60;
            self.db.execute("INSERT INTO credentials(id,user_id,agent_id,token_hash,role,label,expires_at) VALUES(?,?,?,?,'agent',?,?)",params![id(),user_id,agent_id,hash(&token),label,expires_at])?;
            self.db.execute("UPDATE devices SET consumed=1 WHERE device_hash=?",[hash(device_code)])?;
            Ok(Some(AgentToken{token,user_id,agent_id,expires_at}))
        })
    }
    pub fn connections(&self, p: &Principal) -> Result<Vec<ConnectionInfo>> {
        p.human()?;
        Ok(self.db.prepare("SELECT id,label,agent_id,expires_at FROM credentials WHERE user_id=? AND role='agent' AND revoked=0 ORDER BY label,id")?.query_map([&p.user_id],|r|Ok(ConnectionInfo{id:r.get(0)?,label:r.get(1)?,agent_id:r.get(2)?,expires_at:r.get(3)?}))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
    pub fn revoke_connection(&self, p: &Principal, credential_id: &str) -> Result<()> {
        p.human()?;
        self.db.execute(
            "UPDATE credentials SET revoked=1 WHERE id=? AND user_id=?",
            params![credential_id, p.user_id],
        )?;
        Ok(())
    }
    pub fn logout(&self, p: &Principal) -> Result<()> {
        self.db.execute(
            "UPDATE credentials SET revoked=1 WHERE id=?",
            [&p.credential_id],
        )?;
        Ok(())
    }
}
