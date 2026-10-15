//! Portable snapshots of the relay database and its storage key.
use crate::{config, crypto::Vault, store::Store};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, backup::Backup};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub created_at: i64,
    pub database_sha256: String,
    pub key_sha256: String,
}

fn digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn regular_file(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("Missing file: {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("Expected a regular file: {}", path.display())
    }
    Ok(())
}
fn read_key(path: &Path) -> Result<[u8; 32]> {
    regular_file(path)?;
    fs::read(path)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("The backup storage key must contain exactly 32 bytes."))
}
fn check_database(db: &Connection, key: [u8; 32]) -> Result<()> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 1 {
        bail!("This database format is not supported by this Litecom version.")
    }
    let result: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result != "ok" {
        bail!("The database integrity check failed.")
    }
    if db
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        bail!("The database contains broken references.")
    }
    let check: Vec<u8> = db.query_row(
        "SELECT value FROM settings WHERE key='key_check'",
        [],
        |r| r.get(0),
    )?;
    if Vault::new(key).decrypt(&check, "key-check")? != b"litecom" {
        bail!("The storage key does not match the database.")
    }
    Ok(())
}
fn new_directory(path: &Path) -> Result<()> {
    // create_dir, rather than create_dir_all, deliberately refuses existing data.
    fs::create_dir(path)
        .with_context(|| format!("Choose a new, unused folder: {}", path.display()))?;
    config::private_dir(path)
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn create(data: &Path, output: &Path) -> Result<Manifest> {
    if data.join(".restoring").exists() {
        bail!("Restore has not finished. Do not back up this folder yet.")
    }
    let database = data.join("litecom.db");
    regular_file(&database)?;
    let key = read_key(&data.join("storage.key"))?;
    let source = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(5))?;
    new_directory(output)?;
    // A failed copy deliberately has no manifest and cannot be restored.
    let snapshot = output.join("litecom.db");
    config::write_private(&snapshot, b"")?;
    let mut target = Connection::open(&snapshot)?;
    {
        let backup = Backup::new(&source, &mut target)?;
        let started = Instant::now();
        loop {
            match backup.step(256)? {
                rusqlite::backup::StepResult::Done => break,
                _ if started.elapsed() > Duration::from_secs(300) => bail!(
                    "The snapshot could not finish within five minutes. Retry to a new folder when the relay is less busy."
                ),
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
    // The backup is a standalone DB, independent of live WAL sidecars.
    target.execute_batch("PRAGMA journal_mode=DELETE;")?;
    check_database(&target, key)?;
    drop(target);
    fs::File::open(&snapshot)?.sync_all()?;
    config::write_private(&output.join("storage.key"), &key)?;
    let manifest = Manifest {
        format: 1,
        version: env!("CARGO_PKG_VERSION").into(),
        created_at: crate::store::now(),
        database_sha256: digest(&snapshot)?,
        key_sha256: digest(&output.join("storage.key"))?,
    };
    config::write_private(
        &output.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    sync_directory(output)?;
    Ok(manifest)
}

pub fn verify(input: &Path) -> Result<Manifest> {
    for name in ["manifest.json", "litecom.db", "storage.key"] {
        regular_file(&input.join(name))?;
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(input.join("manifest.json"))?)?;
    if manifest.format != 1 {
        bail!("This backup format is not supported by this Litecom version.")
    }
    if digest(&input.join("litecom.db"))? != manifest.database_sha256
        || digest(&input.join("storage.key"))? != manifest.key_sha256
    {
        bail!("The backup checksum does not match. The backup is incomplete or has changed.")
    }
    let db =
        Connection::open_with_flags(input.join("litecom.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_database(&db, read_key(&input.join("storage.key"))?)?;
    Ok(manifest)
}

pub fn restore(input: &Path, data: &Path) -> Result<Manifest> {
    let manifest = verify(input)?;
    new_directory(data)?;
    config::write_private(
        &data.join(".restoring"),
        b"Restore in progress; do not start the relay.",
    )?;
    let database = data.join("litecom.db");
    config::write_private(&database, b"")?;
    fs::copy(input.join("litecom.db"), &database)?;
    let key = read_key(&input.join("storage.key"))?;
    // Verify copied bytes again, so source changes cannot silently alter the copy.
    if digest(&database)? != manifest.database_sha256
        || crate::crypto::hash(key) != manifest.key_sha256
    {
        bail!("The source backup changed during restore. Retry to a new folder.")
    }
    config::write_private(&data.join("storage.key"), &key)?;
    let store = Store::open(&database, key)?;
    store.quarantine_restored_state()?;
    drop(store);
    let db = Connection::open(&database)?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
    check_database(&db, key)?;
    drop(db);
    fs::File::open(database)?.sync_all()?;
    sync_directory(data)?;
    fs::remove_file(data.join(".restoring"))?;
    sync_directory(data)?;
    Ok(manifest)
}
