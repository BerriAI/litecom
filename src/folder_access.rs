//! Request macOS access during setup or from the installed Litecom executable.
use crate::{config, runner::RunnerConfig};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Check {
    workspace: PathBuf,
    complete: bool,
    unavailable: Vec<String>,
    #[serde(default)]
    accessibility: Option<bool>,
    #[serde(default)]
    screen_recording: Option<bool>,
}
fn path(connection: &str) -> Result<PathBuf> {
    Ok(config::ClientConfig::path(connection)?.with_extension("access.json"))
}

/// Recheck existing connections without pairing, changing agent settings,
/// restarting workers, or approving queued requests.
pub async fn approve() -> Result<()> {
    connections()?;
    #[cfg(target_os = "macos")]
    {
        println!(
            "\nComputer permissions\n\nAllow macOS access when prompted: folders, Accessibility, and Screen Recording."
        );
        macos::check_background().await?;
    }
    #[cfg(not(target_os = "macos"))]
    println!(
        "Litecom handles tool approvals within each approved request. Complete any app sign-in or system access prompts in your agent."
    );
    Ok(())
}

fn connections() -> Result<Vec<(String, RunnerConfig)>> {
    let home = config::home()?;
    let mut connections = Vec::new();
    let entries = match std::fs::read_dir(home.join("connections")) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    for entry in entries.into_iter().flatten() {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(connection) = name.to_string_lossy().strip_suffix(".runner.json") {
            config::ClientConfig::path(connection)?;
            connections.push((connection.to_owned(), RunnerConfig::load(connection)?));
        }
    }
    connections.sort_by(|a, b| a.0.cmp(&b.0));
    if connections.is_empty() {
        bail!("No local agent is connected. Connect an agent from your Litecom workspace first.");
    }
    Ok(connections)
}

/// Invoked by the temporary launchd job, so privacy consent belongs to the
/// installed background executable rather than the calling terminal.
pub async fn check_existing() -> Result<()> {
    for (connection, settings) in connections()? {
        println!(
            "{} · {}",
            settings.agent.name(),
            settings.workspace.display()
        );
        prepare(&connection, &settings)?;
        check(&connection, &settings).await?;
        waiting(&connection)?;
    }
    println!(
        "\nPermission check finished. Complete any access still listed above in System Settings. Apps and computer-control tools may also ask for their own access or sign-in. Your connected agents and requests are unchanged."
    );
    Ok(())
}
pub(crate) fn prepare(connection: &str, settings: &RunnerConfig) -> Result<()> {
    if cfg!(target_os = "macos") {
        config::write_private(
            &path(connection)?,
            &serde_json::to_vec(&Check {
                workspace: settings.workspace.clone(),
                complete: false,
                unavailable: vec![],
                accessibility: None,
                screen_recording: None,
            })?,
        )?;
        println!(
            "Allow macOS access when prompted: folders, Accessibility, and Screen Recording. These let approved agents use your files and computer-control tools."
        );
    }
    Ok(())
}

fn folders(workspace: &Path, home: &Path) -> Vec<PathBuf> {
    ["Downloads", "Documents", "Desktop"]
        .into_iter()
        .map(|name| home.join(name))
        .filter(|folder| folder.starts_with(workspace) && folder.is_dir())
        .collect()
}

pub async fn check(connection: &str, settings: &RunnerConfig) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let path = path(connection)?;
    let mut check: Check = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if check.complete || check.workspace != settings.workspace {
        return Ok(());
    }
    // Blocking filesystem calls may wait for a privacy dialog. Run them on a
    // separate thread; opening a directory reads no file contents.
    tokio::task::spawn_blocking(move || -> Result<()> {
        let home = directories::BaseDirs::new()
            .ok_or_else(|| anyhow::anyhow!("Cannot find your home folder."))?;
        for folder in folders(&check.workspace, home.home_dir()) {
            if std::fs::read_dir(&folder).is_err() {
                check.unavailable.push(folder.display().to_string());
            }
        }
        #[cfg(target_os = "macos")]
        {
            let (accessibility, screen) = macos::request();
            check.accessibility = Some(accessibility);
            check.screen_recording = Some(screen);
        }
        check.complete = true;
        config::write_private(&path, &serde_json::to_vec(&check)?)
    })
    .await??;
    Ok(())
}

pub(crate) fn waiting(connection: &str) -> Result<bool> {
    if !cfg!(target_os = "macos") {
        return Ok(false);
    }
    let check: Check = match std::fs::read(path(connection)?) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    if check.complete && !check.unavailable.is_empty() {
        eprintln!(
            "macOS has not allowed access to: {}. Enable access for Litecom in System Settings → Privacy & Security → Files and Folders before requesting those files.",
            check.unavailable.join(", ")
        );
    }
    if check.complete {
        if check.accessibility == Some(false) {
            eprintln!(
                "Computer control needs Accessibility access. Enable Litecom in System Settings → Privacy & Security → Accessibility."
            );
        }
        if check.screen_recording == Some(false) {
            eprintln!(
                "Screen access needs Screen Recording permission. Enable Litecom in System Settings → Privacy & Security → Screen & System Audio Recording. macOS may ask you to restart Litecom."
            );
        }
    }
    Ok(!check.complete)
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use anyhow::Context;
    use core_foundation::{
        base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
    };

    struct Job(String);
    impl Drop for Job {
        fn drop(&mut self) {
            let _ = std::process::Command::new("/bin/launchctl")
                .args(["bootout", &self.0])
                .output();
        }
    }

    pub(super) async fn check_background() -> Result<()> {
        // Replacing the executable atomically leaves running workers alone.
        let executable = crate::service::install_binary()?;
        let home = config::home()?.canonicalize()?;
        run_helper(&executable, &home).await
    }

    async fn run_helper(executable: &Path, home: &Path) -> Result<()> {
        let dir = tempfile::tempdir_in(home)?;
        let result = dir.path().join("result.json");
        let log = dir.path().join("check.log");
        let plist = dir.path().join("check.plist");
        let label = format!("org.litecom.access-{}", crate::crypto::id());
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let job = Job(format!("{domain}/{label}"));
        let text = |path: &Path| crate::service::xml(&path.to_string_lossy());
        let content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array><string>{}</string><string>--home</string><string>{}</string><string>check-access</string><string>{}</string></array>
<key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Background</string>
<key>StandardOutPath</key><string>{}</string>
<key>StandardErrorPath</key><string>{}</string>
</dict></plist>"#,
            text(executable),
            text(home),
            text(&result),
            text(&log),
            text(&log)
        );
        config::write_private(&plist, content.as_bytes())?;
        let output = tokio::process::Command::new("/bin/launchctl")
            .arg("bootstrap")
            .arg(&domain)
            .arg(&plist)
            .output()
            .await?;
        if !output.status.success() {
            bail!(
                "Could not start the permission check: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let outcome = async {
            loop {
                match std::fs::read(&result) {
                    Ok(bytes) => {
                        if let Some(error) = serde_json::from_slice::<Option<String>>(&bytes)? {
                            bail!("{error}");
                        }
                        return Ok(());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                let state = tokio::process::Command::new("/bin/launchctl")
                    .args(["print", &job.0])
                    .output()
                    .await?;
                let state_text = String::from_utf8_lossy(&state.stdout);
                if !state.status.success()
                    || state_text.lines().any(|line| {
                        let line = line.trim();
                        line.strip_prefix("last exit code = ")
                            .is_some_and(|code| code.parse::<i32>().is_ok())
                            || line.starts_with("last terminating signal = ")
                    })
                {
                    // The helper may have finished between the file and job checks.
                    if result.exists() {
                        continue;
                    }
                    bail!(
                        "The permission check stopped before finishing. Run `litecom approve` again."
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        };
        let outcome = tokio::select! {
            result = outcome => result,
            signal = tokio::signal::ctrl_c() => {
                signal.context("Could not listen for cancellation")?;
                Err(anyhow::anyhow!("Permission setup cancelled. Your agents are still running."))
            }
        };
        if let Ok(output) = std::fs::read_to_string(log) {
            print!("{output}");
        }
        outcome
    }

    #[cfg(test)]
    #[tokio::test]
    #[ignore = "requires a signed-in macOS desktop and LITECOM_TEST_BINARY; requests no permissions"]
    async fn permission_helper_reports_errors_and_removes_its_job() {
        let binary = PathBuf::from(std::env::var_os("LITECOM_TEST_BINARY").unwrap());
        let home = tempfile::tempdir().unwrap();
        let error = run_helper(&binary, home.path()).await.unwrap_err();
        assert!(error.to_string().contains("No local agent is connected"));
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
        let jobs = std::process::Command::new("/bin/launchctl")
            .arg("list")
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&jobs.stdout).contains("org.litecom.access-"));
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrustedWithOptions(
            options: core_foundation::dictionary::CFDictionaryRef,
        ) -> bool;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub(super) fn request() -> (bool, bool) {
        let options = CFDictionary::from_CFType_pairs(&[(
            CFString::new("AXTrustedCheckOptionPrompt"),
            CFBoolean::true_value(),
        )]);
        // These APIs request permission only: no screenshot or keyboard input
        // is captured, and no TCC database or global agent setting is changed.
        unsafe {
            let accessibility = AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
            let screen = CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess();
            (accessibility, screen)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_checks_only_standard_folders_inside_the_selected_workspace() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        for name in ["Downloads", "Documents", "Desktop", "Library", "project"] {
            std::fs::create_dir(home.join(name)).unwrap();
        }
        assert_eq!(folders(home, home).len(), 3);
        assert_eq!(
            folders(&home.join("Downloads"), home),
            vec![home.join("Downloads")]
        );
        assert!(folders(&home.join("project"), home).is_empty());
    }
}
