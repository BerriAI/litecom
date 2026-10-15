use crate::crypto::random_bytes;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static HOME_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
pub fn set_home(path: PathBuf) -> Result<()> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    HOME_OVERRIDE
        .set(path)
        .map_err(|_| anyhow::anyhow!("The configuration folder is already set."))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub relay: String,
    pub token: String,
    pub user_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub runner: Option<String>,
    #[serde(default)]
    pub approval_channel: Option<String>,
}
pub fn home() -> Result<PathBuf> {
    if let Some(path) = HOME_OVERRIDE.get() {
        return Ok(path.clone());
    }
    if let Some(path) = std::env::var_os("LITECOM_HOME") {
        return Ok(path.into());
    }
    let dirs = directories::ProjectDirs::from("org", "litecom", "litecom")
        .context("Cannot find your configuration folder. Set LITECOM_HOME.")?;
    Ok(dirs.config_dir().into())
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("The file needs a parent directory.")?;
    private_dir(parent)?;
    let tmp = parent.join(format!(".{}.tmp", crate::crypto::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error.into());
    }
    Ok(())
}
pub fn load_key(dir: &Path) -> Result<[u8; 32]> {
    if dir.join(".restoring").exists() {
        bail!(
            "This restore has not finished. Retry restoring into a new folder before starting the relay."
        );
    }
    private_dir(dir)?;
    let path = dir.join("storage.key");
    if !path.exists() {
        // Never replace a lost key: that would make encrypted data permanently unreadable.
        if dir.join("litecom.db").exists() {
            bail!("storage.key is missing. Restore it from backup before opening this database.");
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(&random_bytes::<32>())?;
        file.sync_all()?;
    }
    fs::read(&path)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("storage.key must contain exactly 32 bytes."))
}
impl ClientConfig {
    pub fn path(connection: &str) -> Result<PathBuf> {
        if connection.is_empty()
            || !connection
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            bail!("Connection names can contain letters, numbers, hyphens, and underscores.");
        }
        Ok(home()?
            .join("connections")
            .join(format!("{connection}.json")))
    }
    pub fn load() -> Result<Self> {
        Self::load_named("default")
    }
    pub fn load_named(connection: &str) -> Result<Self> {
        Ok(serde_json::from_slice(
            &fs::read(Self::path(connection)?)
                .context("Run `litecom setup --relay <your server URL>` first.")?,
        )?)
    }
    pub fn save(&self) -> Result<()> {
        self.save_named("default")
    }
    pub fn save_named(&self, connection: &str) -> Result<()> {
        write_private(&Self::path(connection)?, &serde_json::to_vec_pretty(self)?)
    }
}
pub fn validate_url(value: &str, local_allowed: bool) -> Result<url::Url> {
    let url = url::Url::parse(value)?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if url.scheme() != "https" && !(local_allowed && local && url.scheme() == "http") {
        bail!("Use an HTTPS server URL (HTTP is allowed only on localhost).");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("Use the server's base URL without credentials, path, query, or fragment.");
    }
    Ok(url)
}
