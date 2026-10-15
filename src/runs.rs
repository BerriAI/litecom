//! Private local run records. An OS lock distinguishes a live worker from an
//! interrupted run, even after a crash or PID reuse.
use crate::{client::Client, config, model::WorkItem, runner::RunnerConfig};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
pub struct Run {
    pub task_id: String,
    pub agent_id: String,
    pub harness: String,
    pub workspace: PathBuf,
    pub started_at: i64,
    pub status: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub directory: PathBuf,
}
pub(crate) struct RunGuard {
    run: Run,
    lock: File,
}
impl RunGuard {
    pub(crate) fn start(
        dir: &Path,
        client: &Client,
        settings: &RunnerConfig,
        work: &WorkItem,
    ) -> Result<Self> {
        config::write_private(&dir.join("active.lock"), b"")?;
        let lock = File::options()
            .read(true)
            .write(true)
            .open(dir.join("active.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        let this = Self {
            run: Run {
                task_id: work.task_id.clone(),
                agent_id: client.config.agent_id.clone(),
                harness: settings.agent.name().into(),
                workspace: settings.workspace.clone(),
                started_at: crate::store::now(),
                status: "running".into(),
                session_id: None,
                directory: dir.into(),
            },
            lock,
        };
        this.save()?;
        Ok(this)
    }
    fn save(&self) -> Result<()> {
        config::write_private(
            &self.run.directory.join("run.json"),
            &serde_json::to_vec_pretty(&self.run)?,
        )
    }
    pub(crate) fn finish(&mut self, status: &str) -> Result<()> {
        self.run.status = status.into();
        self.save()
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        if self.run.status == "running" {
            self.run.status = "interrupted".into();
            let _ = self.save();
        }
        // Explicitly unlock: a concurrent process spawn can inherit the open
        // file until exec, delaying a lock released only by closing this file.
        let _ = fs2::FileExt::unlock(&self.lock);
    }
}
fn active(dir: &Path) -> Result<bool> {
    let file = match File::options()
        .read(true)
        .write(true)
        .open(dir.join("active.lock"))
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            fs2::FileExt::unlock(&file)?;
            Ok(false)
        }
        Err(e)
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Ok(true)
        }
        Err(e) => Err(e.into()),
    }
}
fn session_id(dir: &Path) -> Option<String> {
    if let Ok(value) = fs::read_to_string(dir.join("session-id")) {
        return Some(value.trim().into());
    }
    let file = File::open(dir.join("agent-output.json")).ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Ok(event) = serde_json::from_str::<Value>(&line)
            && let Some(id) = event["thread_id"].as_str().or(event["session_id"].as_str())
        {
            return Some(id.into());
        }
    }
    None
}
pub fn list_in(root: &Path) -> Result<Vec<Run>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut runs = vec![];
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let mut run: Run = match fs::read(dir.join("run.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("Cannot read {}", dir.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Earlier versions retained only failed diagnostics. Do not infer
                // that one of these directories is a live session.
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(id) = name
                    .get(..36)
                    .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                else {
                    continue;
                };
                Run {
                    task_id: id.into(),
                    agent_id: String::new(),
                    harness: "Legacy run".into(),
                    workspace: PathBuf::new(),
                    started_at: 0,
                    status: "diagnostics only".into(),
                    session_id: None,
                    directory: dir.clone(),
                }
            }
            Err(e) => return Err(e.into()),
        };
        run.directory = dir.clone();
        if run.status == "running" && !active(&dir)? {
            run.status = "interrupted".into();
        }
        run.session_id = session_id(&dir);
        runs.push(run);
    }
    runs.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then(a.task_id.cmp(&b.task_id))
    });
    Ok(runs)
}
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}
pub fn list(json: bool, only_active: bool) -> Result<()> {
    let runs = list_in(&config::home()?.join("runs"))?
        .into_iter()
        .filter(|r| !only_active || r.status == "running")
        .collect::<Vec<_>>();
    if json {
        println!("{}", serde_json::to_string_pretty(&runs)?);
        return Ok(());
    }
    if runs.is_empty() {
        println!(
            "No {}local runs. Approved requests appear here when they start.",
            if only_active { "active " } else { "" }
        );
    }
    for run in runs {
        println!(
            "{}  {} · {}\n  {}",
            run.task_id,
            clean(&run.harness),
            run.status,
            clean(&run.workspace.display().to_string())
        );
        if let Some(id) = run.session_id {
            println!("  Session: {}", clean(&id));
        }
    }
    Ok(())
}
fn select(root: &Path, id: Option<&str>) -> Result<Run> {
    let mut runs = list_in(root)?;
    if let Some(id) = id {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            bail!("Use a request ID from `litecom runs`.");
        }
        runs.retain(|r| r.task_id.starts_with(id));
        if runs.len() > 1 {
            bail!("More than one run matches. Use its full request ID from `litecom runs`.");
        }
    } else if let Some(index) = runs.iter().position(|r| r.status == "running") {
        return Ok(runs.remove(index));
    }
    runs.into_iter()
        .next()
        .context("No local run found. Use `litecom runs` to list requests.")
}
/// Follow disk output only. This command never attaches input to the agent.
pub async fn logs(id: Option<&str>, follow: bool, raw: bool) -> Result<()> {
    let run = select(&config::home()?.join("runs"), id)?;
    eprintln!("{} · {} · {}", run.task_id, clean(&run.harness), run.status);
    if let Some(id) = &run.session_id {
        eprintln!("Session: {}", clean(id));
    }
    let names = ["agent-output.json", "agent-events.jsonl", "agent-error.log"];
    let mut offsets = [0u64; 3];
    let mut fragments = [Vec::new(), Vec::new(), Vec::new()];
    loop {
        let running = active(&run.directory)?;
        for (i, name) in names.iter().enumerate() {
            let mut file = match File::open(run.directory.join(name)) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            file.seek(SeekFrom::Start(offsets[i]))?;
            let mut bytes = vec![];
            file.read_to_end(&mut bytes)?;
            offsets[i] += bytes.len() as u64;
            fragments[i].extend_from_slice(&bytes);
            while let Some(end) = fragments[i].iter().position(|b| *b == b'\n') {
                let line = fragments[i].drain(..=end).collect::<Vec<_>>();
                print_line(&String::from_utf8_lossy(&line), raw);
            }
            if (!follow || !running) && !fragments[i].is_empty() {
                print_line(
                    &String::from_utf8_lossy(&std::mem::take(&mut fragments[i])),
                    raw,
                );
            }
        }
        std::io::stdout().flush()?;
        if !follow || !running {
            break;
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
    }
    Ok(())
}
fn print_line(line: &str, raw: bool) {
    let text = if !raw {
        serde_json::from_str::<Value>(line).ok().and_then(|event| {
            let kind = event["type"].as_str()?;
            if kind == "thread.started" {
                Some(format!("Codex session: {}", event["thread_id"].as_str()?))
            } else {
                event.get("item").map(|item| {
                    format!(
                        "{}: {}",
                        item["type"].as_str().unwrap_or(kind),
                        item.get("text")
                            .or_else(|| item.get("command"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| item.to_string())
                    )
                })
            }
        })
    } else {
        None
    };
    println!("{}", clean(text.as_deref().unwrap_or(line).trim_end()));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_lock_prevents_stale_running_labels_and_paths_cannot_escape() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("run");
        config::private_dir(&dir).unwrap();
        config::write_private(&dir.join("active.lock"), b"").unwrap();
        let lock = File::options()
            .read(true)
            .write(true)
            .open(dir.join("active.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&lock).unwrap();
        let run = Run {
            task_id: crate::crypto::id(),
            agent_id: "fixture".into(),
            harness: "Codex".into(),
            workspace: root.path().into(),
            started_at: 1,
            status: "running".into(),
            session_id: None,
            directory: dir.clone(),
        };
        config::write_private(&dir.join("run.json"), &serde_json::to_vec(&run).unwrap()).unwrap();
        fs::write(
            dir.join("agent-output.json"),
            "{\"type\":\"thread.started\",\"thread_id\":\"session-1\"}\n",
        )
        .unwrap();
        let listed = list_in(root.path()).unwrap();
        assert_eq!(listed[0].status, "running");
        assert_eq!(listed[0].session_id.as_deref(), Some("session-1"));
        // A clone models the descriptor briefly inherited during process spawn.
        // Finishing the guard must unlock even while that descriptor is open.
        let inherited = lock.try_clone().unwrap();
        let guard = RunGuard { run, lock };
        drop(guard);
        assert!(!active(&dir).unwrap());
        let mut stale: Run =
            serde_json::from_slice(&fs::read(dir.join("run.json")).unwrap()).unwrap();
        assert_eq!(stale.status, "interrupted");
        stale.status = "running".into();
        config::write_private(&dir.join("run.json"), &serde_json::to_vec(&stale).unwrap()).unwrap();
        assert_eq!(list_in(root.path()).unwrap()[0].status, "interrupted");
        drop(inherited);
        assert!(select(root.path(), Some("../../other")).is_err());
    }
}
