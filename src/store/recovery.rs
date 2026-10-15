use super::*;

impl Store {
    /// A backup may predate a revoked credential or an already-executed action.
    /// Keep account data/history, but never resume authority from that snapshot.
    pub(crate) fn quarantine_restored_state(&self) -> Result<()> {
        self.transaction(|| {
            self.db.execute_batch("UPDATE credentials SET revoked=1;
                DELETE FROM otp; DELETE FROM local_pairings; DELETE FROM devices; DELETE FROM channel_challenges;
                DELETE FROM login_states;
                DELETE FROM oauth_links; DELETE FROM channel_actions; DELETE FROM deliveries; DELETE FROM channels;
                DELETE FROM cloud_jobs; DELETE FROM tool_permissions;
                UPDATE agents SET config=NULL,last_seen_at=NULL;
                DELETE FROM contacts WHERE status!='blocked';")?;
            let profiles=self.db.prepare("SELECT id FROM profiles")?.query_map([],|r|r.get::<_,String>(0))?
                .collect::<std::result::Result<Vec<_>,_>>()?;
            for profile in profiles {
                let invite=secret();
                self.db.execute("UPDATE profiles SET invite_hash=?,invite=? WHERE id=?",params![hash(&invite),self.vault.seal(&invite,&format!("invite:{profile}"))?,profile])?;
            }
            for mut task in self.task_rows("SELECT id,data FROM tasks",[])? {
                if !task.view.status.terminal() {
                    task.view.status=Status::Failed;
                    task.view.receipt=Some(Receipt{outcome:Outcome::Uncertain,
                        summary:"This request was recovered from a backup and will not run again. Check the original agent or cloud session for completed effects before sending a new request.".into(),urls:vec![]});
                }
                task.view.revoked_at=Some(self.time());
                task.view.response=None;
                task.view.permissions.clear();
                if let Some(progress)=&mut task.view.execution {
                    progress.message="Recovered from backup. Inspect this session directly; Litecom has disconnected it and will not resume it.".into();
                    progress.needs_attention=true;
                }
                Self::clear_lease(&mut task);
                self.save(&mut task,"restored")?;
            }
            Ok(())
        })
    }
}
