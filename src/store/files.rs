//! Immutable file snapshots, encrypted in bounded chunks in the same SQLite
//! database as their metadata. Backups remain a single database plus its key.
use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, Write};

// An I/O buffer size, not a file-size limit.
const CHUNK: usize = 1024 * 1024;

pub struct Upload {
    pub artifact: Artifact,
    spool: std::fs::File,
    vault: Vault,
    digest: Sha256,
    chunks: u64,
    pending: Vec<u8>,
}
impl Upload {
    pub fn write(&mut self, mut bytes: &[u8]) -> Result<()> {
        self.digest.update(bytes);
        self.artifact.size = self
            .artifact
            .size
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| {
                Error::Invalid("The file exceeds this platform's size representation.".into())
            })?;
        while !bytes.is_empty() {
            let take = (CHUNK - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.pending.len() == CHUNK {
                self.flush_chunk()?;
            }
        }
        Ok(())
    }
    fn flush_chunk(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let encrypted = self.vault.encrypt(
            &self.pending,
            &format!("artifact-chunk:{}:{}", self.artifact.id, self.chunks),
        )?;
        self.spool
            .write_all(&(encrypted.len() as u32).to_le_bytes())?;
        self.spool.write_all(&encrypted)?;
        self.pending.clear();
        self.chunks += 1;
        Ok(())
    }
}

impl Store {
    fn can_upload(&self, p: &Principal, task_id: &str, lease: Option<&str>) -> Result<()> {
        let task = self.task(task_id)?;
        self.owner(p, &task)?;
        if p.role == Role::Agent {
            self.lease(p, &task, lease.unwrap_or(""))?;
            conflict(
                matches!(task.view.status, Status::Preparing | Status::Executing),
                "Files can only be prepared while this agent holds a work lease.",
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
        Ok(())
    }
    pub fn begin_upload(
        &self,
        p: &Principal,
        task_id: &str,
        lease: Option<&str>,
        name: &str,
        media_type: &str,
    ) -> Result<Upload> {
        self.can_upload(p, task_id, lease)?;
        validate(
            !name.is_empty()
                && !name
                    .chars()
                    .any(|c| c.is_control() || c == '/' || c == '\\'),
            "Use a filename without folders or control characters.",
        )?;
        validate(
            !media_type.trim().is_empty()
                && media_type
                    .bytes()
                    .all(|c| c.is_ascii() && !c.is_ascii_control()),
            "Provide a valid media type.",
        )?;
        Ok(Upload {
            artifact: Artifact {
                id: id(),
                task_id: task_id.into(),
                name: name.into(),
                media_type: media_type.into(),
                size: 0,
                sha256: String::new(),
            },
            spool: tempfile::tempfile()?,
            vault: self.vault.clone(),
            digest: Sha256::new(),
            chunks: 0,
            pending: Vec::with_capacity(CHUNK),
        })
    }
    pub fn finish_upload(
        &self,
        p: &Principal,
        lease: Option<&str>,
        mut upload: Upload,
    ) -> Result<Artifact> {
        upload.flush_chunk()?;
        upload.artifact.sha256 = hex::encode(upload.digest.clone().finalize());
        upload.spool.rewind()?;
        self.transaction(|| {
            let artifact = &upload.artifact;
            self.can_upload(p, &artifact.task_id, lease)?;
            let existing = self
                .db
                .prepare("SELECT id,metadata FROM artifacts WHERE task_id=?")?
                .query_map([&artifact.task_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (id, data) in existing {
                let old: Artifact = self.vault.open(&data, &format!("artifact-meta:{id}"))?;
                if old.sha256 == artifact.sha256
                    && old.name == artifact.name
                    && old.media_type == artifact.media_type
                {
                    return Ok(old);
                }
            }
            // Empty data distinguishes the chunk format from legacy encrypted blobs.
            self.db.execute(
                "INSERT INTO artifacts(id,task_id,size,metadata,data) VALUES(?,?,?,?,?)",
                params![
                    artifact.id,
                    artifact.task_id,
                    artifact.size,
                    self.vault
                        .seal(artifact, &format!("artifact-meta:{}", artifact.id))?,
                    Vec::<u8>::new()
                ],
            )?;
            for sequence in 0..upload.chunks {
                let mut length = [0; 4];
                upload.spool.read_exact(&mut length)?;
                let length = u32::from_le_bytes(length) as usize;
                require(length <= CHUNK + 29, "The temporary upload is invalid.")?;
                let mut bytes = vec![0; length];
                upload.spool.read_exact(&mut bytes)?;
                self.db.execute(
                    "INSERT INTO artifact_chunks(artifact_id,sequence,data) VALUES(?,?,?)",
                    params![artifact.id, sequence, bytes],
                )?;
            }
            Ok(artifact.clone())
        })
    }
    pub fn upload(
        &self,
        p: &Principal,
        task_id: &str,
        lease: Option<&str>,
        name: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<Artifact> {
        let mut upload = self.begin_upload(p, task_id, lease, name, media_type)?;
        upload.write(bytes)?;
        self.finish_upload(p, lease, upload)
    }
    pub fn download_metadata(&self, p: &Principal, artifact_id: &str) -> Result<Artifact> {
        let artifact = self.artifact(artifact_id)?;
        let task = self.task(&artifact.task_id)?;
        self.authorized(p, &task)?;
        require(
            task.view.revoked_at.is_none() && !task.view.expired(self.time()),
            "This download expired or was revoked.",
        )?;
        if p.user_id != task.view.owner_id {
            require(
                task.view.status == Status::Completed
                    && task
                        .view
                        .draft
                        .as_ref()
                        .is_some_and(|d| d.attachments.iter().any(|a| a.id == artifact_id)),
                "This file has not been shared with you.",
            )?;
        }
        Ok(artifact)
    }
    pub fn download_chunk(
        &self,
        p: &Principal,
        artifact_id: &str,
        sequence: u64,
    ) -> Result<Option<Vec<u8>>> {
        self.download_metadata(p, artifact_id)?;
        let chunk: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT data FROM artifact_chunks WHERE artifact_id=? AND sequence=?",
                params![artifact_id, sequence],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(chunk) = chunk {
            return self
                .vault
                .decrypt(&chunk, &format!("artifact-chunk:{artifact_id}:{sequence}"))
                .map(Some);
        }
        if sequence == 0 {
            let legacy: Vec<u8> = self.db.query_row(
                "SELECT data FROM artifacts WHERE id=?",
                [artifact_id],
                |r| r.get(0),
            )?;
            if !legacy.is_empty() {
                return self
                    .vault
                    .decrypt(&legacy, &format!("artifact:{artifact_id}"))
                    .map(Some);
            }
        }
        Ok(None)
    }
    // Convenience for in-process callers and tests; HTTP downloads stream chunks.
    #[cfg(test)]
    pub fn download(&self, p: &Principal, artifact_id: &str) -> Result<(Artifact, Vec<u8>)> {
        let artifact = self.download_metadata(p, artifact_id)?;
        let mut bytes = vec![];
        let mut sequence = 0;
        while let Some(chunk) = self.download_chunk(p, artifact_id, sequence)? {
            bytes.extend(chunk);
            sequence += 1;
        }
        require(
            bytes.len() as u64 == artifact.size && hash(&bytes) == artifact.sha256,
            "The file failed its integrity check.",
        )?;
        Ok((artifact, bytes))
    }
}
