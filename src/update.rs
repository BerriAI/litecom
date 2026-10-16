//! Explicit, verified upgrades. Connections and service definitions are read-only.
use crate::{
    client::Client,
    config::{self, ClientConfig},
    service,
};
use anyhow::{Context, Result, bail};
use clap::Args;
use semver::Version;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[derive(Args)]
pub struct Options {
    /// Show the newest published release without changing anything.
    #[arg(long)]
    pub check: bool,
    /// Install a specific published version (for example 0.1.0-alpha.24).
    #[arg(long)]
    pub to: Option<String>,
    /// Seconds to wait for active requests before leaving the installation alone.
    #[arg(long, default_value_t = 600)]
    pub wait: u64,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
}
fn version(tag: &str) -> Result<Version> {
    Ok(Version::parse(tag.strip_prefix('v').unwrap_or(tag))?)
}
fn target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => bail!("No published Litecom download supports this platform. Update from source."),
    }
}
fn executable() -> &'static str {
    if cfg!(windows) {
        "litecom.exe"
    } else {
        "litecom"
    }
}
fn archive_name() -> Result<String> {
    Ok(format!(
        "litecom-{}.{}",
        target()?,
        if cfg!(windows) { "zip" } else { "tar.gz" }
    ))
}
fn select_release(releases: Vec<Release>, name: &str) -> Result<Version> {
    releases
        .into_iter()
        .filter(|r| {
            !r.draft
                && r.assets.iter().any(|a| a.name == name)
                && r.assets.iter().any(|a| a.name == format!("{name}.sha256"))
        })
        .filter_map(|r| version(&r.tag_name).ok())
        .max()
        .context("No complete published release is available for this computer.")
}
async fn latest(http: &reqwest::Client, to: Option<&str>) -> Result<Version> {
    let url = if let Some(to) = to {
        format!(
            "https://api.github.com/repos/BerriAI/litecom/releases/tags/v{}",
            version(to)?
        )
    } else {
        "https://api.github.com/repos/BerriAI/litecom/releases?per_page=100".into()
    };
    let response = http
        .get(url)
        .send()
        .await
        .context("Cannot reach GitHub. Check your network and retry; nothing was changed.")?
        .error_for_status()?;
    let releases = if to.is_some() {
        vec![response.json::<Release>().await?]
    } else {
        response.json().await?
    };
    select_release(releases, &archive_name()?)
}
fn verify_checksum(bytes: &[u8], checksum: &str) -> Result<()> {
    let expected = checksum.split_whitespace().next().unwrap_or("");
    if expected.len() != 64
        || !expected.bytes().all(|c| c.is_ascii_hexdigit())
        || !hex::encode(Sha256::digest(bytes)).eq_ignore_ascii_case(expected)
    {
        bail!("The download failed its SHA-256 check. Nothing was installed; retry the update.");
    }
    Ok(())
}
fn binary_version(path: &Path) -> Result<Version> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("Cannot run {}", path.display()))?;
    let text = String::from_utf8(output.stdout)?;
    if !output.status.success() {
        bail!("The downloaded executable could not start.");
    }
    version(
        text.trim()
            .strip_prefix("litecom ")
            .context("Not a Litecom executable.")?,
    )
}
async fn download(http: &reqwest::Client, release: &Version, dir: &Path) -> Result<PathBuf> {
    let name = archive_name()?;
    let base = format!("https://github.com/BerriAI/litecom/releases/download/v{release}/{name}");
    let checksum = http
        .get(format!("{base}.sha256"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let bytes = http
        .get(base)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    verify_checksum(&bytes, &checksum)?;
    let archive = dir.join(name);
    fs::write(&archive, bytes)?;
    let folder = format!("litecom-{}", target()?);
    #[cfg(not(windows))]
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(dir)
        .arg(format!("{folder}/{}", executable()))
        .output()?;
    #[cfg(windows)]
    let output = Command::new("powershell.exe").args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
        &format!("$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath '{}' -DestinationPath '{}'", archive.to_string_lossy().replace('\'', "''"), dir.to_string_lossy().replace('\'', "''"))]).output()?;
    if !output.status.success() {
        bail!(
            "Cannot unpack the Litecom download: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let binary = dir.join(folder).join(executable());
    if binary_version(&binary)? != *release {
        bail!("The downloaded executable does not match the selected release.");
    }
    Ok(binary)
}
struct Connection {
    name: String,
    client: Client,
    restart: bool,
}
fn connections(home: &Path) -> Result<Vec<Connection>> {
    let mut result = vec![];
    if !home.join("connections").try_exists()? {
        return Ok(result);
    }
    for entry in fs::read_dir(home.join("connections"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = name.strip_suffix(".runner.json") else {
            continue;
        };
        let config = ClientConfig::load_named(name)?;
        result.push(Connection {
            name: name.into(),
            client: Client::new(config)?,
            restart: service::running_owned(name)?,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}
// Do not chmod the containing bin directory: it may be shared with other tools.
fn replace(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("Missing installation directory.")?;
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut File::open(source)?, &mut staged)?;
    let permissions = fs::metadata(destination)
        .or_else(|_| fs::metadata(source))?
        .permissions();
    staged.as_file().set_permissions(permissions)?;
    staged.as_file().sync_all()?;
    #[cfg(not(windows))]
    staged.persist(destination)?;
    #[cfg(windows)]
    {
        // Windows permits renaming a mapped executable but cannot overwrite it.
        // Keep a running MCP process alive; its old image can be removed later.
        let old = parent.join(format!(".litecom-previous-{}.exe", uuid::Uuid::new_v4()));
        let existed = destination.try_exists()?;
        if existed {
            fs::rename(destination, &old)?;
        }
        if let Err(error) = staged.persist(destination) {
            if existed {
                fs::rename(&old, destination)
                    .context("Could not restore the previous executable.")?;
            }
            return Err(error.into());
        }
        if existed {
            let _ = fs::remove_file(old);
        }
    }
    Ok(())
}
fn destinations(current: &Path, installed: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = vec![current.canonicalize()?];
    if installed.try_exists()? {
        let installed = installed.canonicalize()?;
        if !paths.contains(&installed) {
            paths.push(installed);
        }
    }
    // Invoking the managed MCP binary should also update an existing PATH CLI.
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let path = directory.join(executable());
            if path.is_file() {
                let path = path.canonicalize()?;
                if !paths.contains(&path) && binary_version(&path).is_ok() {
                    paths.push(path);
                }
                break;
            }
        }
    }
    Ok(paths)
}
async fn pause(connections: &[Connection], id: &str) -> Result<usize> {
    let results = futures_util::future::join_all(connections.iter().map(|connection| async move {
        let value: Value = connection
            .client
            .post("/api/work/update", json!({"id":id}))
            .await
            .with_context(|| {
                format!(
                    "Cannot pause {} safely. The server must support Litecom updates.",
                    connection.name
                )
            })?;
        Ok(value["active"]
            .as_u64()
            .context("Invalid update status from the server.")? as usize)
    }))
    .await;
    results.into_iter().sum()
}
async fn resume(connections: &[Connection], id: &str) {
    for connection in connections {
        if let Err(error) = connection
            .client
            .post::<Value>("/api/work/update", json!({"id":id,"release":true}))
            .await
        {
            eprintln!(
                "{} will resume within three minutes if the server cannot be reached: {error}",
                connection.name
            );
        }
    }
}
async fn restart(names: &[String]) -> Result<()> {
    let mut failures = vec![];
    for name in names {
        let mut started = false;
        for _ in 0..20 {
            if service::status(name)? {
                started = true;
                break;
            }
            let _ = service::start_existing(name);
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if !started && !service::status(name)? {
            failures.push(name.clone());
        }
    }
    if !failures.is_empty() {
        bail!(
            "Could not restart {}. Run `litecom --connection NAME service start` for each listed connection.",
            failures.join(", ")
        );
    }
    Ok(())
}
async fn apply(
    source: &Path,
    paths: &[PathBuf],
    connections: &[Connection],
    wait: Duration,
    home: &Path,
) -> Result<()> {
    // Complete backup and write-permission checks before stopping any service.
    let backups = tempfile::tempdir()?;
    for (i, path) in paths.iter().enumerate() {
        fs::copy(path, backups.path().join(i.to_string()))?;
        tempfile::NamedTempFile::new_in(path.parent().context("Missing installation directory.")?)?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut stopped = vec![];
    let result: Result<()> = tokio::select! { result = async {
        let deadline = Instant::now() + wait;
        let mut announced = false;
        loop {
            let remote_active = pause(connections, &id).await?;
            let local_active = crate::runs::list_in(&home.join("runs"))?.iter().any(|r| r.status == "running");
            if remote_active == 0 && !local_active { break; }
            if !announced { println!("Waiting for active requests to finish. Queued requests will wait for the update."); announced = true; }
            if Instant::now() >= deadline { bail!("Active requests are still running. Nothing was installed or stopped. Retry after they finish, or use `litecom update --wait SECONDS`. Run updates from your own terminal/chat, not from a request the worker is still executing."); }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        for connection in connections.iter().filter(|c| c.restart) {
            // Refresh the pause before every OS operation.
            if pause(connections, &id).await? != 0 { bail!("Work became active during the update; retry after it finishes."); }
            stopped.push(connection.name.clone());
            service::stop(&connection.name)?;
        }
        let install: Result<()> = (|| {
            for path in paths { replace(source, path)?; }
            for path in paths { if fs::read(path)? != fs::read(source)? { bail!("The installed version did not verify."); } }
            Ok(())
        })();
        if let Err(error) = install {
            let mut rollback_errors = vec![];
            for (i, path) in paths.iter().enumerate() {
                if let Err(e) = replace(&backups.path().join(i.to_string()), path) { rollback_errors.push(format!("{}: {e}", path.display())); }
            }
            if !rollback_errors.is_empty() { bail!("{error}. Could not restore: {}", rollback_errors.join("; ")); }
            return Err(error).context("The previous executables were restored.");
        }
        Ok(())
    } => result,
    _ = tokio::signal::ctrl_c() => Err(anyhow::anyhow!("Update cancelled. Existing connections are preserved.")),
    };
    let restarted = restart(&stopped).await;
    resume(connections, &id).await;
    match (result, restarted) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(restart)) => Err(error).context(format!("Also: {restart}")),
    }
}
pub async fn run(options: Options) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("litecom/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(300))
        .build()?;
    println!("Checking for Litecom updates…");
    let release = latest(&http, options.to.as_deref()).await?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    println!("Installed: {current} · Latest: {release}");
    if options.check {
        return Ok(());
    }
    let home = config::home()?;
    config::private_dir(&home)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join("update.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("Another Litecom update is already running.")?;
    if release < current {
        bail!("This installation is newer than that published release. It was kept.");
    }
    let paths = destinations(&std::env::current_exe()?, &service::binary()?)?;
    if paths
        .iter()
        .all(|p| binary_version(p).is_ok_and(|v| v == release))
    {
        println!("Litecom is up to date.");
        return Ok(());
    }
    let connections = connections(&home)?;
    let temp = tempfile::tempdir()?;
    println!("Downloading Litecom {release}…");
    let source = download(&http, &release, temp.path()).await?;
    apply(
        &source,
        &paths,
        &connections,
        Duration::from_secs(options.wait),
        &home,
    )
    .await?;
    println!("Updated to Litecom {release}. Existing connections and settings are preserved.");
    println!(
        "Running background workers restarted. Open a new agent chat when you need the updated MCP tools; existing chats can continue using the CLI."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksums_fail_closed() {
        let sum = hex::encode(Sha256::digest(b"valid"));
        assert!(verify_checksum(b"valid", &format!("{sum}  archive.tar.gz\n")).is_ok());
        assert!(verify_checksum(b"tampered", &sum).is_err());
        assert!(verify_checksum(b"valid", "").is_err());
    }
    #[test]
    fn latest_includes_prereleases_but_requires_complete_assets() {
        let releases = serde_json::from_value(json!([
            {"tag_name":"v0.1.0-alpha.9","draft":false,"assets":[{"name":"package"},{"name":"package.sha256"}]},
            {"tag_name":"v0.1.0-alpha.24","draft":false,"assets":[{"name":"package"},{"name":"package.sha256"}]},
            {"tag_name":"v0.1.0-alpha.25","draft":true,"assets":[{"name":"package"},{"name":"package.sha256"}]},
            {"tag_name":"v0.1.0-alpha.26","draft":false,"assets":[{"name":"package"}]}
        ])).unwrap();
        assert_eq!(
            select_release(releases, "package").unwrap().to_string(),
            "0.1.0-alpha.24"
        );
        assert!(version("../../other").is_err());
    }
    #[test]
    fn replacement_is_atomic_and_preserves_directory_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("new");
        let destination = tmp.path().join("old");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();
        replace(&source, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(replace(&tmp.path().join("missing"), &destination).is_err());
        assert_eq!(fs::read(destination).unwrap(), b"new");
    }

    #[tokio::test]
    async fn applying_updates_both_copies_without_touching_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("download");
        let cli = dir.path().join("cli");
        let managed = dir.path().join("managed");
        let config = dir.path().join("saved.json");
        fs::write(&source, b"new release").unwrap();
        fs::write(&cli, b"old cli").unwrap();
        fs::write(&managed, b"old managed").unwrap();
        fs::write(&config, b"saved connection and environment").unwrap();
        apply(
            &source,
            &[cli.clone(), managed.clone()],
            &[],
            Duration::ZERO,
            dir.path(),
        )
        .await
        .unwrap();
        assert_eq!(fs::read(cli).unwrap(), b"new release");
        assert_eq!(fs::read(managed).unwrap(), b"new release");
        assert_eq!(
            fs::read(config).unwrap(),
            b"saved connection and environment"
        );
    }

    #[tokio::test]
    async fn failed_replacement_restores_all_copies() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::write(&first, b"first old version").unwrap();
        fs::write(&second, b"second old version").unwrap();
        assert!(
            apply(
                &dir.path().join("missing"),
                &[first.clone(), second.clone()],
                &[],
                Duration::ZERO,
                dir.path()
            )
            .await
            .is_err()
        );
        assert_eq!(fs::read(first).unwrap(), b"first old version");
        assert_eq!(fs::read(second).unwrap(), b"second old version");
    }

    #[tokio::test]
    async fn active_run_timeout_does_not_replace_a_binary() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("runs/one");
        fs::create_dir_all(&run).unwrap();
        let lock = File::create(run.join("active.lock")).unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        fs::write(run.join("run.json"), serde_json::to_vec(&json!({
            "task_id":"fixture", "agent_id":"fixture", "harness":"fixture", "workspace":dir.path(),
            "started_at":1, "status":"running"
        })).unwrap()).unwrap();
        let source = dir.path().join("download");
        let dest = dir.path().join("installed");
        fs::write(&source, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();
        assert!(
            apply(
                &source,
                std::slice::from_ref(&dest),
                &[],
                Duration::ZERO,
                dir.path()
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("still running")
        );
        assert_eq!(fs::read(dest).unwrap(), b"old");
    }
}
