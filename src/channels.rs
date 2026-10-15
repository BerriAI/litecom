use crate::{
    auth::OtpChallenge,
    crypto::{equal, id},
    error::{Error, Result, require, validate},
    model::Principal,
    store::Store,
};
use rand::Rng;
use rusqlite::{OptionalExtension, params};

pub fn email_address(value: &str) -> Result<String> {
    let email = value.trim();
    validate(
        email.len() <= 254 && !email.contains(['\r', '\n']),
        "Enter a valid email address.",
    )?;
    let address: lettre::Address = email
        .parse()
        .map_err(|_| Error::Invalid("Enter a valid email address.".into()))?;
    Ok(address.to_string())
}
impl Store {
    pub(crate) fn verified_channel(
        &self,
        p: &Principal,
        kind: &str,
        destination: &str,
    ) -> Result<()> {
        p.human()?;
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM channels WHERE user_id=? AND kind=?",
                params![p.user_id, kind],
                |r| r.get(0),
            )
            .optional()?;
        let channel = existing.unwrap_or_else(id);
        self.db
            .execute("DELETE FROM channel_actions WHERE channel_id=?", [&channel])?;
        self.db.execute("UPDATE deliveries SET state='cancelled' WHERE channel_id=? AND state IN ('pending','sending')",[&channel])?;
        self.db.execute(
            "UPDATE deliveries SET remote_id=NULL WHERE channel_id=?",
            [&channel],
        )?;
        self.db.execute("INSERT INTO channels(id,user_id,kind,destination,verified) VALUES(?,?,?,?,1) ON CONFLICT(user_id,kind) DO UPDATE SET destination=excluded.destination,verified=1",params![channel,p.user_id,kind,self.vault.seal(&destination,&format!("channel:{channel}"))?])?;
        Ok(())
    }
    pub fn begin_email(&self, p: &Principal, email: &str) -> Result<(OtpChallenge, String)> {
        p.human()?;
        let email = email_address(email)?;
        self.transaction(||{
            let destination_key=self.vault.fingerprint("email",email.as_bytes());
            let count:i64=self.db.query_row("SELECT COUNT(*) FROM channel_challenges WHERE (user_id=? OR destination_key=?) AND expires_at>?",params![p.user_id,destination_key,self.time()-3000],|r|r.get(0))?;
            if count>=5{return Err(Error::RateLimited);}
            let recent:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM channel_challenges WHERE user_id=? AND expires_at>?)",params![p.user_id,self.time()+540],|r|r.get(0))?;
            if recent{return Err(Error::RateLimited);}
            let challenge=id();
            let code=format!("{:06}",rand::thread_rng().gen_range(0..1_000_000));
            self.db.execute("INSERT INTO channel_challenges(id,user_id,credential_id,destination_key,code_hash,data,expires_at) VALUES(?,?,?,?,?,?,?)",params![challenge,p.user_id,p.credential_id,destination_key,self.vault.fingerprint(&format!("email:{challenge}"),code.as_bytes()),self.vault.seal(&email,&format!("email:{challenge}"))?,self.time()+600])?;
            Ok((OtpChallenge{id:challenge,expires_in:600},code))
        })
    }
    pub fn verify_email(&self, p: &Principal, challenge: &str, code: &str) -> Result<()> {
        p.human()?;
        // Return an invalid result after committing the attempt count.
        let valid=self.transaction(||{
            let row=self.db.query_row("SELECT user_id,credential_id,code_hash,data,expires_at,attempts,used FROM channel_challenges WHERE id=?",[challenge],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,bool>(6)?))).optional()?;
            let Some((user,credential,expected,data,expires,attempts,used))=row else{return Ok(false)};
            require(user==p.user_id && credential==p.credential_id,"Use the browser that requested this email code.")?;
            if used || expires<=self.time() || attempts>=5{return Ok(false);}
            self.db.execute("UPDATE channel_challenges SET attempts=attempts+1 WHERE id=?",[challenge])?;
            if !equal(&expected,&self.vault.fingerprint(&format!("email:{challenge}"),code.as_bytes())){return Ok(false);}
            let email:String=self.vault.open(&data,&format!("email:{challenge}"))?;
            self.verified_channel(p,"email",&email)?;
            self.db.execute("UPDATE channel_challenges SET used=1 WHERE id=?",[challenge])?;
            Ok(true)
        })?;
        validate(
            valid,
            "That email code is incorrect or expired. Request another code and try again.",
        )
    }
}
