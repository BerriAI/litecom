//! Connect ordinary agent sessions as well as Litecom's receiving sessions.
//! Native CLIs own their configuration formats; Cursor's JSONC uses an OSS CST.
use crate::{
    config,
    runner::{NativeAgent, ProcessGroup, RunnerConfig, find_executable},
};
use anyhow::{Context, Result, bail};
use jsonc_parser::{ParseOptions, cst::CstRootNode};
use rmcp::{ServiceExt, transport::TokioChildProcess};
use serde_json::{Map, Value, json};
use std::{
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};
use tokio::process::Command;

pub struct Connection {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub name: String,
}
impl Connection {
    pub fn new(connection: &str, executable: &Path) -> Result<Self> {
        config::ClientConfig::path(connection)?;
        Ok(Self {
            executable: executable.into(),
            arguments: vec![
                "--home".into(),
                config::home()?.canonicalize()?.to_string_lossy().into(),
                "--connection".into(),
                connection.into(),
                "mcp".into(),
            ],
            name: if connection == "default" {
                "litecom".into()
            } else {
                format!("litecom_{connection}")
            },
        })
    }
    fn entry(&self, agent: NativeAgent) -> Value {
        if agent == NativeAgent::OpenCode {
            let command = std::iter::once(self.executable.to_string_lossy().into_owned())
                .chain(self.arguments.clone())
                .collect::<Vec<_>>();
            json!({"type":"local","command":command})
        } else if matches!(agent, NativeAgent::Codex | NativeAgent::Claude) {
            json!({"type":"stdio","command":self.executable,"args":self.arguments})
        } else {
            json!({"command":self.executable,"args":self.arguments})
        }
    }
    pub async fn probe(&self) -> Result<()> {
        let mut command = Command::new(&self.executable);
        command
            .args(&self.arguments)
            .env_remove("LITECOM_TOKEN")
            .stderr(Stdio::null());
        let transport = TokioChildProcess::new(command)?;
        tokio::time::timeout(Duration::from_secs(20), async {
            let client = rmcp::model::ClientConfig::default()
                .serve(transport)
                .await?;
            let tools = client.list_tools(Default::default()).await?.tools;
            let valid = ["litecom_ask", "litecom_receive", "litecom_contacts"]
                .iter()
                .all(|name| tools.iter().any(|tool| tool.name == *name));
            client.cancel().await?;
            if !valid {
                bail!("The saved Litecom command did not expose the expected tools.");
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("Checking Litecom's tools timed out. Your connection is saved; retry setup.")??;
        Ok(())
    }
}

fn map(value: Option<&Value>) -> Result<Map<String, Value>> {
    match value {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        _ => bail!("The agent's MCP configuration is not an object. Its settings were preserved."),
    }
}
fn equivalent(actual: &Value, expected: &Value) -> bool {
    expected
        .as_object()
        .is_some_and(|fields| fields.iter().all(|(k, v)| actual.get(k) == Some(v)))
        && actual
            .get("env")
            .is_none_or(|v| v.is_null() || v == &json!({}))
        && actual
            .get("environment")
            .is_none_or(|v| v.is_null() || v == &json!({}))
        && actual
            .get("env_vars")
            .is_none_or(|v| v.is_null() || v == &json!([]))
}
fn enabled(entry: &Value) -> bool {
    entry["enabled"] != false && entry["disabled"] != true
}

/// Reuse the exact connection. Never overwrite another server or work around a
/// deliberately disabled matching entry by creating an enabled duplicate.
fn choose_name(
    base: &str,
    servers: &Map<String, Value>,
    expected: &Value,
) -> Result<(String, bool)> {
    for (name, entry) in servers {
        if equivalent(entry, expected) {
            if !enabled(entry) {
                bail!(
                    "The matching Litecom server '{name}' is disabled in your agent. Enable it there, then retry setup."
                );
            }
            return Ok((name.clone(), false));
        }
    }
    for suffix in 0..100 {
        let name = if suffix == 0 {
            base.into()
        } else {
            format!("{base}_{}", suffix + 1)
        };
        if !servers.contains_key(&name) {
            return Ok((name, true));
        }
    }
    bail!("Too many conflicting Litecom server names. Your agent settings were preserved.")
}

async fn run(executable: &Path, cwd: &Path, args: &[String]) -> Result<Output> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(cwd)
        .env("PWD", cwd)
        .env_remove("LITECOM_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .context("Could not start your agent's configuration command.")?;
    let _group = ProcessGroup(child.id());
    tokio::time::timeout(Duration::from_secs(30), child.wait_with_output()).await
        .context("Your agent's configuration command timed out. Your Litecom account is saved; retry setup.")?
        .context("Your agent's configuration command stopped unexpectedly.")
}
fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
async fn read_cli(settings: &RunnerConfig) -> Result<Map<String, Value>> {
    let args = match settings.agent {
        NativeAgent::Codex => strings(&["mcp", "list", "--json"]),
        NativeAgent::OpenCode => strings(&["--pure", "debug", "config"]),
        NativeAgent::Hermes => strings(&["config", "get", "mcp_servers", "--json"]),
        NativeAgent::OpenClaw => strings(&["config", "get", "mcp", "--json"]),
        _ => unreachable!(),
    };
    let output = run(&settings.executable, &settings.workspace, &args).await?;
    if !output.status.success() {
        // Missing keys are different from invalid config, a locked gateway, or
        // an older CLI. Only an explicit missing-key report means an empty map.
        let message = String::from_utf8_lossy(&output.stderr).to_string()
            + &String::from_utf8_lossy(&output.stdout);
        let missing = match settings.agent {
            NativeAgent::Hermes => message.contains("Config key not set: mcp_servers"),
            NativeAgent::OpenClaw => message.contains("Config path not found: mcp"),
            _ => false,
        };
        if missing {
            return Ok(Map::new());
        }
        bail!(
            "Could not read {}'s MCP settings. Check its configuration or update its CLI, then retry setup. No server was replaced.",
            settings.agent.name()
        );
    }
    // Never put raw config (which can contain credentials) into an error.
    let value: Value = serde_json::from_slice(&output.stdout)
        .context("Your agent did not return valid configuration JSON. No server was replaced.")?;
    match settings.agent {
        NativeAgent::Codex => {
            let mut servers = Map::new();
            for server in value
                .as_array()
                .context("Codex did not return an MCP server list.")?
            {
                let name = server["name"]
                    .as_str()
                    .context("Codex returned an unnamed MCP server.")?;
                let mut entry = server["transport"]
                    .as_object()
                    .cloned()
                    .context("Codex returned an MCP server without transport settings.")?;
                let enabled = server["enabled"]
                    .as_bool()
                    .context("Codex returned an MCP server without its enabled state.")?;
                entry.insert("enabled".into(), json!(enabled));
                if servers.insert(name.into(), Value::Object(entry)).is_some() {
                    bail!(
                        "Codex returned duplicate MCP server names. Its settings were preserved."
                    );
                }
            }
            Ok(servers)
        }
        NativeAgent::OpenCode => {
            let mcp = value.get("mcp");
            map(mcp.and_then(|v| v.get("servers")).or(mcp))
        }
        NativeAgent::OpenClaw => map(value.get("servers")),
        NativeAgent::Hermes => map(Some(&value)),
        _ => unreachable!(),
    }
}

fn claude_config_path() -> Result<PathBuf> {
    if let Some(directory) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Ok(PathBuf::from(directory).join(".claude.json"));
    }
    Ok(directories::BaseDirs::new()
        .context("Cannot find your home directory.")?
        .home_dir()
        .join(".claude.json"))
}

/// Human-readable CLI output cannot preserve argument boundaries. Read the
/// documented JSON for exact values; let the CLI check effective scope/status.
fn read_claude(path: &Path, workspace: &Path, expected: &Value) -> Result<Map<String, Value>> {
    let (_, value) = read_document(path, false)?;
    let mut servers = map(value.get("mcpServers"))?;
    for ancestor in workspace.ancestors() {
        let project = ancestor.join(".mcp.json");
        if project.is_file() {
            let (_, value) = read_document(&project, false)?;
            reserve_project(
                &mut servers,
                map(value.get("mcpServers"))?,
                expected,
                &project,
            )?;
        }
        if let Some(local) = value
            .get("projects")
            .and_then(|p| p.get(ancestor.to_string_lossy().as_ref()))
        {
            reserve_project(&mut servers, map(local.get("mcpServers"))?, expected, path)?;
        }
    }
    Ok(servers)
}

fn reserve_project(
    servers: &mut Map<String, Value>,
    project: Map<String, Value>,
    expected: &Value,
    path: &Path,
) -> Result<()> {
    for (name, value) in project {
        if equivalent(&value, expected) {
            bail!(
                "Litecom is already configured in {}. Move that entry to your agent's user configuration before retrying setup.",
                path.display()
            );
        }
        if servers.get(&name).is_some_and(|v| equivalent(v, expected)) {
            bail!(
                "{} overrides the existing Litecom connection. Resolve that project override before retrying setup.",
                path.display()
            );
        }
        servers.insert(name, value);
    }
    Ok(())
}

fn claude_connected(output: &Output, expected: &Value) -> bool {
    let text = String::from_utf8_lossy(&output.stdout);
    let fields = |prefix: &str| {
        text.lines()
            .filter_map(|line| line.trim().strip_prefix(prefix))
            .collect::<Vec<_>>()
    };
    output.status.success()
        && fields("Scope: ") == ["User config (available in all your projects)"]
        && fields("Type: ") == ["stdio"]
        && fields("Command: ") == [expected["command"].as_str().unwrap_or_default()]
        && matches!(
            fields("Status: ").as_slice(),
            ["✓ Connected" | "✔ Connected"]
        )
}

fn read_document(path: &Path, comments: bool) -> Result<(String, Value)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => "{}\n".into(),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Could not read {}. Its settings were preserved.",
                    path.display()
                )
            });
        }
    };
    let value = if comments {
        CstRootNode::parse(&text, &ParseOptions::default())
            .ok()
            .and_then(|root| root.to_serde_value())
    } else {
        serde_json::from_str(&text).ok()
    };
    let value = value.filter(Value::is_object).with_context(|| {
        format!(
            "{} is not a valid configuration object. Its settings were preserved.",
            path.display()
        )
    })?;
    Ok((text, value))
}

/// Cursor has no native CLI add command. Preserve comments/formatting using
/// jsonc-parser and save atomically, with a backup and concurrent-change check.
fn write_cursor(path: &Path, before: &str, name: &str, entry: &Value) -> Result<()> {
    let root = CstRootNode::parse(before, &ParseOptions::default())?;
    let object = root
        .object_value()
        .context("Cursor MCP configuration must be an object.")?;
    if object
        .properties()
        .iter()
        .filter(|p| p.decoded_name().as_deref() == Some("mcpServers"))
        .count()
        > 1
    {
        bail!("Cursor has duplicate mcpServers keys. Its settings were preserved.");
    }
    let servers = object
        .object_value_or_create("mcpServers")
        .context("Cursor mcpServers must be an object.")?;
    if servers.get(name).is_some() {
        bail!("Cursor's MCP server name is already in use.");
    }
    let command = entry["command"].as_str().context("Missing MCP command")?;
    let args = entry["args"]
        .as_array()
        .context("Missing MCP arguments")?
        .iter()
        .map(|v| v.as_str().map(String::from).context("Invalid MCP argument"))
        .collect::<Result<Vec<_>>>()?;
    use jsonc_parser::cst::CstInputValue;
    servers.append(
        name,
        CstInputValue::Object(vec![
            ("command".into(), command.into()),
            ("args".into(), args.into()),
        ]),
    );
    let target = if path.is_symlink() {
        path.canonicalize()?
    } else {
        path.to_path_buf()
    };
    let path = target.as_path();
    let parent = path
        .parent()
        .context("Missing Cursor configuration folder")?;
    std::fs::create_dir_all(parent)?;
    let existing = path.try_exists()?;
    let permissions = if existing {
        Some(std::fs::metadata(path)?.permissions())
    } else {
        None
    };
    if existing && std::fs::read_to_string(path)? != before {
        bail!("Cursor's settings changed during setup. Retry setup to preserve those changes.");
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    temporary.write_all(root.to_string().as_bytes())?;
    temporary.as_file().sync_all()?;
    if existing {
        let backup = parent.join(format!("mcp.json.litecom-backup-{}", crate::crypto::id()));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(before.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(backup)?;
    }
    if existing {
        if std::fs::read_to_string(path)? != before {
            bail!("Cursor's settings changed during setup. Retry setup.");
        }
        temporary.persist(path)?;
    } else {
        temporary.persist_noclobber(path)?;
    }
    Ok(())
}

pub async fn install(settings: &RunnerConfig, connection: &Connection) -> Result<String> {
    let expected = connection.entry(settings.agent);
    let (path, before, mut servers) = match settings.agent {
        NativeAgent::Claude => {
            let path = claude_config_path()?;
            let servers = read_claude(&path, &settings.workspace, &expected)?;
            (Some(path), None, servers)
        }
        NativeAgent::Pi | NativeAgent::Cursor => {
            let home = directories::BaseDirs::new()
                .context("Cannot find your home directory.")?
                .home_dir()
                .to_owned();
            let path = if settings.agent == NativeAgent::Pi {
                std::env::var_os("PI_CODING_AGENT_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".pi/agent"))
                    .join("mcp.json")
            } else {
                home.join(".cursor/mcp.json")
            };
            let (before, value) = read_document(&path, settings.agent == NativeAgent::Cursor)?;
            let servers = map(value.get("mcpServers"))?;
            (Some(path), Some(before), servers)
        }
        _ => (None, None, read_cli(settings).await?),
    };
    // A project entry can shadow the same user-level name. Avoid collisions
    // rather than granting trust to a project or replacing its configuration.
    if matches!(settings.agent, NativeAgent::Pi | NativeAgent::Cursor) {
        let directory = if settings.agent == NativeAgent::Pi {
            ".pi"
        } else {
            ".cursor"
        };
        for ancestor in settings.workspace.ancestors() {
            let project = ancestor.join(directory).join("mcp.json");
            if Some(&project) == path.as_ref() {
                continue;
            }
            if project.is_file() {
                let (_, value) = read_document(&project, settings.agent == NativeAgent::Cursor)?;
                reserve_project(
                    &mut servers,
                    map(value.get("mcpServers"))?,
                    &expected,
                    &project,
                )?;
            }
        }
    }
    let (name, add) = choose_name(&connection.name, &servers, &expected)?;
    if add {
        if settings.agent == NativeAgent::Cursor {
            write_cursor(
                path.as_ref().unwrap(),
                before.as_ref().unwrap(),
                &name,
                &expected,
            )?;
        } else {
            let executable = if settings.agent == NativeAgent::Pi {
                find_executable("pi").context("pi is not installed.")?
            } else {
                settings.executable.clone()
            };
            let mut args = match settings.agent {
                NativeAgent::Codex => strings(&["mcp", "add", &name, "--"]),
                NativeAgent::Claude => strings(&["mcp", "add", "--scope", "user", &name, "--"]),
                NativeAgent::Pi => strings(&["mcp", "add", &name, "--"]),
                NativeAgent::OpenCode => {
                    let help = run(
                        &executable,
                        &settings.workspace,
                        &strings(&["mcp", "add", "--help"]),
                    )
                    .await?;
                    let mut args = strings(&["--pure", "mcp", "add", &name]);
                    if String::from_utf8_lossy(&help.stdout).contains("--global") {
                        args.push("--global".into());
                    }
                    args.push("--".into());
                    args
                }
                NativeAgent::Hermes => strings(&[
                    "config",
                    "set",
                    &format!("mcp_servers.{name}"),
                    &expected.to_string(),
                ]),
                NativeAgent::OpenClaw => strings(&["mcp", "set", &name, &expected.to_string()]),
                _ => unreachable!(),
            };
            if matches!(
                settings.agent,
                NativeAgent::Pi | NativeAgent::OpenCode | NativeAgent::Codex | NativeAgent::Claude
            ) {
                args.push(connection.executable.to_string_lossy().into());
                args.extend(connection.arguments.clone());
            }
            let output = run(&executable, &settings.workspace, &args).await?;
            if !output.status.success() {
                if settings.agent == NativeAgent::OpenClaw {
                    bail!(
                        "OpenClaw could not save the MCP connection. Its gateway may own the configuration: stop it through its service manager, retry setup, then restart it. Managed read-only configuration must be changed by its owner."
                    );
                }
                bail!(
                    "{} could not save the Litecom connection. Your phone account is saved; check the agent's configuration and retry setup.",
                    settings.agent.name()
                );
            }
        }
    }
    let saved = if let Some(path) = path {
        map(read_document(&path, settings.agent == NativeAgent::Cursor)?
            .1
            .get("mcpServers"))?
    } else {
        read_cli(settings).await?
    };
    if !saved
        .get(&name)
        .is_some_and(|v| equivalent(v, &expected) && enabled(v))
    {
        bail!(
            "{} did not save the expected Litecom connection. Setup is incomplete; your phone account is saved.",
            settings.agent.name()
        );
    }
    if settings.agent == NativeAgent::Cursor {
        let output = run(
            &settings.executable,
            &settings.workspace,
            &strings(&["mcp", "enable", &name]),
        )
        .await?;
        if !output.status.success() {
            bail!(
                "The connection is saved, but Cursor could not enable it. Run `agent mcp enable {name}`, then retry setup."
            );
        }
    }
    if settings.agent == NativeAgent::Claude {
        let output = run(
            &settings.executable,
            &settings.workspace,
            &strings(&["mcp", "get", &name]),
        )
        .await?;
        if !claude_connected(&output, &expected) {
            bail!(
                "The connection is saved, but Claude did not confirm it connected from your user settings. Run `claude mcp get {name}` to check for a disabled connection, project override, or managed policy, then retry setup."
            );
        }
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Value {
        json!({"command":"/a path/litecom","args":["--connection","test","mcp"]})
    }

    #[test]
    fn names_preserve_other_servers_and_disabled_choices() {
        let mut servers = Map::new();
        servers.insert("litecom".into(), json!({"command":"someone-else"}));
        assert_eq!(
            choose_name("litecom", &servers, &entry()).unwrap(),
            ("litecom_2".into(), true)
        );
        servers.insert("litecom_2".into(), entry());
        assert_eq!(
            choose_name("litecom", &servers, &entry()).unwrap(),
            ("litecom_2".into(), false)
        );
        servers.get_mut("litecom_2").unwrap()["enabled"] = json!(false);
        assert!(choose_name("litecom", &servers, &entry()).is_err());

        let same_display_different_arguments = json!({
            "command":"/a path/litecom", "args":["--connection test", "mcp"]
        });
        assert!(!equivalent(&same_display_different_arguments, &entry()));
    }

    #[test]
    fn claude_checks_exact_user_config_and_reserves_project_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("claude.json");
        let workspace = temp.path().join("project");
        std::fs::create_dir(&workspace).unwrap();
        let expected = json!({"type":"stdio","command":"/bin/litecom","args":["a b", "c"]});
        let wrong_args = json!({"type":"stdio","command":"/bin/litecom","args":["a", "b c"]});
        let config = json!({
            "mcpServers":{"litecom":wrong_args},
            "projects":{workspace.to_string_lossy().as_ref():{
                "mcpServers":{"litecom_2":{"command":"other-project-server"}}
            }}
        });
        std::fs::write(&path, config.to_string()).unwrap();
        let servers = read_claude(&path, &workspace, &expected).unwrap();
        assert_eq!(
            choose_name("litecom", &servers, &expected).unwrap(),
            ("litecom_3".into(), true)
        );

        let mut config = config;
        config["mcpServers"]["litecom"] = expected.clone();
        std::fs::write(&path, config.to_string()).unwrap();
        std::fs::write(
            workspace.join(".mcp.json"),
            r#"{"mcpServers":{"litecom":{"command":"other"}}}"#,
        )
        .unwrap();
        assert!(read_claude(&path, &workspace, &expected).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), config.to_string());
    }

    #[cfg(unix)]
    #[test]
    fn claude_does_not_treat_disabled_or_wrong_scope_as_connected() {
        use std::os::unix::process::ExitStatusExt;
        let expected = json!({"command":"/bin/litecom"});
        let success = "litecom:\n  Scope: User config (available in all your projects)\n  Status: ✓ Connected\n  Type: stdio\n  Command: /bin/litecom\n";
        let output = |text: String| Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: text.into_bytes(),
            stderr: vec![],
        };
        assert!(claude_connected(&output(success.into()), &expected));
        assert!(claude_connected(
            &output(success.replace('✓', "✔")),
            &expected
        ));
        for changed in [
            success.replace(
                "✓ Connected",
                "⊘ Disabled for this project (re-enable via /mcp)",
            ),
            success.replace(
                "User config (available in all your projects)",
                "Local config (private to you in this project)",
            ),
            success.replace("✓ Connected", "✘ Failed to connect"),
            success.replace("/bin/litecom", "/bin/other"),
        ] {
            assert!(!claude_connected(&output(changed), &expected));
        }
    }

    #[test]
    fn cursor_preserves_comments_other_servers_and_private_backup() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("mcp.json");
        let before = "{\n  // Keep this comment.\n  \"mcpServers\": {\"other\": {\"command\": \"other-tool\"}},\n  \"custom\": true,\n}\n";
        std::fs::write(&path, before).unwrap();
        write_cursor(&path, before, "litecom", &entry()).unwrap();
        let (text, value) = read_document(&path, true).unwrap();
        assert!(text.contains("// Keep this comment."));
        assert_eq!(value["mcpServers"]["other"]["command"], "other-tool");
        assert_eq!(value["mcpServers"]["litecom"], entry());
        assert_eq!(value["custom"], true);
        let backup = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("mcp.json.litecom-backup")
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(backup).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn cursor_refuses_invalid_conflicting_or_concurrently_changed_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("mcp.json");
        for before in [
            "not json",
            "{\"mcpServers\":{},\"mcpServers\":{}}",
            "{\"mcpServers\":false}",
            "{\"mcpServers\":{\"litecom\":{}}}",
        ] {
            std::fs::write(&path, before).unwrap();
            assert!(write_cursor(&path, before, "litecom", &entry()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        }
        std::fs::write(&path, "{\"new\":true}").unwrap();
        assert!(write_cursor(&path, "{}", "litecom", &entry()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{\"new\":true}");
    }

    #[cfg(unix)]
    #[test]
    fn cursor_preserves_symlinked_dotfiles() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("dotfiles.json");
        let path = temp.path().join("mcp.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        write_cursor(&path, "{}", "litecom", &entry()).unwrap();
        assert!(path.is_symlink());
        assert_eq!(
            read_document(&target, true).unwrap().1["mcpServers"]["litecom"],
            entry()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_cli_setup_checks_saved_output_and_does_not_repeat_writes() {
        use std::os::unix::fs::PermissionsExt;
        for agent in [
            NativeAgent::Codex,
            NativeAgent::OpenCode,
            NativeAgent::Hermes,
            NativeAgent::OpenClaw,
        ] {
            for saved in [true, false] {
                let temp = tempfile::tempdir().unwrap();
                let executable = temp.path().join("native-agent");
                let script = r#"#!/bin/sh
if [ "$1" = --pure ]; then shift; fi
if [ "$1" = mcp ] && [ "$2" = add ] && [ "$3" = --help ]; then echo --global; exit 0; fi
if [ "$1" = debug ] || [ "$2" = get ] || [ "$2" = list ]; then cat actual.json; exit 0; fi
printf '%s\n' "$@" > arguments.txt
echo write >> writes.txt
if [ ! -f fail-save ]; then cp expected.json actual.json; fi
"#;
                std::fs::write(&executable, script).unwrap();
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                let settings = RunnerConfig {
                    agent,
                    executable,
                    workspace: temp.path().into(),
                    arguments: agent.default_arguments(),
                    mcp: None,
                };
                let connection = Connection {
                    executable: "/a path/litecom".into(),
                    arguments: vec!["--connection".into(), "test".into(), "mcp".into()],
                    name: "litecom".into(),
                };
                let wrap = |servers: Value| {
                    match agent {
                    NativeAgent::Codex => Value::Array(servers.as_object().unwrap().iter().map(|(name, transport)| json!({"name":name,"enabled":true,"transport":transport})).collect()),
                    NativeAgent::OpenCode => json!({"mcp":servers}),
                    NativeAgent::OpenClaw => json!({"servers":servers}),
                    _ => servers,
                }
                };
                std::fs::write(
                    temp.path().join("actual.json"),
                    wrap(json!({"litecom":{"command":"unrelated"}})).to_string(),
                )
                .unwrap();
                std::fs::write(temp.path().join("expected.json"),wrap(json!({"litecom":{"command":"unrelated"},"litecom_2":connection.entry(agent)})).to_string()).unwrap();
                if !saved {
                    std::fs::write(temp.path().join("fail-save"), "").unwrap();
                }
                let result = install(&settings, &connection).await;
                if saved {
                    assert_eq!(result.unwrap(), "litecom_2");
                    assert_eq!(install(&settings, &connection).await.unwrap(), "litecom_2");
                } else {
                    assert!(result.is_err());
                }
                assert_eq!(
                    std::fs::read_to_string(temp.path().join("writes.txt")).unwrap(),
                    "write\n"
                );
                let arguments = std::fs::read_to_string(temp.path().join("arguments.txt")).unwrap();
                assert!(arguments.contains("litecom_2"));
                assert!(arguments.contains("/a path/litecom"));
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_or_malformed_config_read_never_adds_a_server() {
        use std::os::unix::fs::PermissionsExt;
        for response in [
            "exit 1",
            "echo not-json",
            "echo '{}'",
            "echo '[{\"name\":\"one\",\"transport\":{}}]'",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let executable = temp.path().join("codex");
            std::fs::write(&executable, format!("#!/bin/sh\nif [ \"$2\" = list ]; then {response}; exit; fi\ntouch unexpected-write\n")).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let settings = RunnerConfig {
                agent: NativeAgent::Codex,
                executable,
                workspace: temp.path().into(),
                arguments: vec![],
                mcp: None,
            };
            let connection = Connection {
                executable: "/bin/litecom".into(),
                arguments: vec!["mcp".into()],
                name: "litecom".into(),
            };
            assert!(install(&settings, &connection).await.is_err());
            assert!(!temp.path().join("unexpected-write").exists());
        }
    }
}
