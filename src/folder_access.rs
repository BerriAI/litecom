//! Ask macOS for folder access from the installed worker, during setup.
use crate::{config, runner::RunnerConfig};
use anyhow::Result;
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
    use core_foundation::{
        base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
    };

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
