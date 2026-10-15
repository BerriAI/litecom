//! Remove owned client files without deleting working folders, downloads or relay data.
use crate::{
    agent_setup,
    client::Client,
    config::{self, ClientConfig},
    runner::RunnerConfig,
    service,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| {
            format!(
                "Could not remove {}. Close agent sessions using Litecom, then rerun uninstall.",
                path.display()
            )
        }),
    }
}

fn connections(home: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let entries = match fs::read_dir(home.join("connections")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(names),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(name) = name
            .strip_suffix(".runner.json")
            .or_else(|| name.strip_suffix(".json"))
        {
            ClientConfig::path(name)?;
            names.insert(name.into());
        }
    }
    Ok(names)
}

fn receipt_path() -> Result<PathBuf> {
    let base = if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(path)
    } else if cfg!(windows) {
        PathBuf::from(
            std::env::var_os("LOCALAPPDATA").context("Cannot find the installation receipt.")?,
        )
    } else {
        directories::BaseDirs::new()
            .context("Cannot find your home directory.")?
            .home_dir()
            .join(".config")
    };
    Ok(base.join("litecom/litecom-receipt.json"))
}

fn remove_installation(receipt: &Path) -> Result<()> {
    if !receipt.try_exists()? {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(receipt)?)?;
    let binary = if cfg!(windows) {
        "litecom.exe"
    } else {
        "litecom"
    };
    if value["source"]["app_name"] != "litecom"
        || value["source"]["owner"] != "BerriAI"
        || value["binaries"] != serde_json::json!([binary])
    {
        bail!(
            "The installation receipt is not a recognized Litecom installation. It was preserved."
        );
    }
    let prefix = PathBuf::from(
        value["install_prefix"]
            .as_str()
            .context("Missing installation directory")?,
    );
    if !prefix.is_absolute() {
        bail!("The installation receipt needs an absolute directory.");
    }
    remove_file(&prefix.join(binary))?;
    remove_file(receipt)?;
    // Shared bin directories and shell PATH settings may belong to other tools.
    Ok(())
}

pub async fn run(all: bool) -> Result<()> {
    let home = config::home()?;
    let names = connections(&home)?.into_iter().collect::<Vec<_>>();
    println!("Litecom connections · {}", home.display());
    let selected = if all || names.is_empty() {
        names
    } else {
        let labels = names
            .iter()
            .map(|name| {
                let settings = RunnerConfig::path(name)
                    .ok()
                    .and_then(|p| fs::read(p).ok())
                    .and_then(|bytes| serde_json::from_slice::<RunnerConfig>(&bytes).ok());
                match settings {
                    Some(settings) => format!(
                        "{} · {} ({name})",
                        settings.agent.name(),
                        settings.workspace.display()
                    ),
                    None => format!("{name} · sending connection"),
                }
            })
            .collect::<Vec<_>>();
        println!("Space to select · Enter to uninstall selected · Esc to cancel");
        let choices = dialoguer::MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Which connections do you want to uninstall?")
            .items(&labels).interact_opt()
            .context("Run uninstall in a terminal to choose connections, or use --all to remove every connection.")?;
        let selected = choices
            .unwrap_or_default()
            .into_iter()
            .map(|i| names[i].clone())
            .collect::<Vec<_>>();
        if selected.is_empty() {
            println!("Nothing selected. No connections changed.");
            return Ok(());
        }
        selected
    };
    let mut failures = Vec::new();
    for name in selected {
        let result: Result<()> = async {
            service::remove_owned(&name)?;
            let runner = RunnerConfig::path(&name)?;
            if runner.try_exists()? {
                // Cleanup still works when the old working folder has disappeared.
                let settings: RunnerConfig = serde_json::from_slice(&fs::read(&runner)?)?;
                let connection = agent_setup::Connection::new(&name, &service::binary()?)?;
                agent_setup::remove(&settings, &connection).await?;
            }
            let path = ClientConfig::path(&name)?;
            if path.try_exists()? {
                Client::new(ClientConfig::load_named(&name)?)?
                    .disconnect()
                    .await?;
            }
            remove_file(&path)?;
            remove_file(&runner)?;
            remove_file(&service::log_path(&name)?)?;
            println!("Disconnected {name}.");
            Ok(())
        }
        .await;
        if let Err(error) = result {
            failures.push(format!("{name}: {error:#}"));
        }
    }
    if !failures.is_empty() {
        bail!(
            "Uninstall needs another try:\n{}\nCompleted removals are saved. Rerun the same command after resolving these issues.",
            failures.join("\n")
        );
    }
    if !connections(&home)?.is_empty() {
        println!(
            "Selected connections uninstalled. Your other Litecom connections are still installed."
        );
        return Ok(());
    }
    remove_file(&service::binary()?)?;
    remove_installation(&receipt_path()?)?;
    // Remove empty owned directories only. Never recursively remove --home.
    for folder in ["connections", "logs", "bin"] {
        let _ = fs::remove_dir(home.join(folder));
    }
    let _ = fs::remove_dir(&home);
    println!(
        "Litecom uninstalled. Your agents, working files, downloads and workspace account are kept."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_removes_only_the_installed_binary_and_is_repeatable() {
        let temp = tempfile::tempdir().unwrap();
        let binary = if cfg!(windows) {
            "litecom.exe"
        } else {
            "litecom"
        };
        let receipt = temp.path().join("receipt.json");
        fs::write(temp.path().join(binary), "binary").unwrap();
        fs::write(temp.path().join("other-tool"), "keep").unwrap();
        fs::write(&receipt, serde_json::json!({"source":{"app_name":"litecom","owner":"BerriAI"},"binaries":[binary],"install_prefix":temp.path()}).to_string()).unwrap();
        remove_installation(&receipt).unwrap();
        remove_installation(&receipt).unwrap();
        assert!(!temp.path().join(binary).exists());
        assert_eq!(
            fs::read_to_string(temp.path().join("other-tool")).unwrap(),
            "keep"
        );
    }
}
