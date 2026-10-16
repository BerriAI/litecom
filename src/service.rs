//! Per-user startup integration. No administrator account or system daemon required.
use crate::{config, runner::RunnerConfig};
use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn label(connection: &str) -> Result<String> {
    config::ClientConfig::path(connection)?; // Validate before putting it in a service name.
    Ok(format!("org.litecom.{connection}"))
}
pub(crate) fn binary() -> Result<PathBuf> {
    Ok(config::home()?.join("bin").join(if cfg!(windows) {
        "litecom.exe"
    } else {
        "litecom"
    }))
}
pub fn install_binary() -> Result<PathBuf> {
    let dest = binary()?;
    let source = std::env::current_exe()?;
    copy_binary(&source, &dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dest)
}
fn copy_binary(source: &Path, dest: &Path) -> Result<bool> {
    if source == dest {
        return Ok(false);
    }
    let bytes = std::fs::read(source)?;
    match std::fs::read(dest) {
        Ok(installed) if installed == bytes => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Cannot check the installed Litecom executable."),
    }
    // Repeating setup must not replace an identical executable. Windows can
    // read a running executable but refuses to replace it while it is in use.
    config::write_private(dest, &bytes).context(
        "Cannot update the installed Litecom executable. Stop its background workers and close agent sessions using Litecom, then retry setup.",
    )?;
    Ok(true)
}
pub fn log_path(connection: &str) -> Result<PathBuf> {
    config::ClientConfig::path(connection)?;
    Ok(config::home()?
        .join("logs")
        .join(format!("{connection}.log")))
}
fn checked(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .context("Could not run your system's background service manager.")?;
    if !output.status.success() {
        bail!(
            "Background startup failed: {}. Your connection is saved. Run `litecom worker` to listen in this terminal.",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn user_home() -> Result<PathBuf> {
    Ok(directories::BaseDirs::new()
        .context("Cannot find your home folder.")?
        .home_dir()
        .into())
}

#[cfg(target_os = "macos")]
fn service_path(connection: &str) -> Result<PathBuf> {
    Ok(user_home()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label(connection)?)))
}
#[cfg(target_os = "linux")]
fn service_path(connection: &str) -> Result<PathBuf> {
    Ok(user_home()?
        .join(".config/systemd/user")
        .join(format!("{}.service", label(connection)?)))
}

pub(crate) fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
/// Pure generator, also exercised on other platforms in tests.
pub fn launch_agent(
    connection: &str,
    executable: &Path,
    home: &Path,
    log: &Path,
    path: &str,
) -> Result<String> {
    let label = label(connection)?;
    let args = [
        executable.to_string_lossy().into_owned(),
        "--connection".into(),
        connection.into(),
        "worker".into(),
    ]
    .iter()
    .map(|v| format!("<string>{}</string>", xml(v)))
    .collect::<String>();
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array>{args}</array>
<key>EnvironmentVariables</key><dict><key>LITECOM_HOME</key><string>{home}</string><key>PATH</key><string>{path}</string></dict>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>10</integer>
<key>ProcessType</key><string>Background</string>
<key>StandardOutPath</key><string>{log}</string>
<key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#,
        home = xml(&home.to_string_lossy()),
        path = xml(path),
        log = xml(&log.to_string_lossy())
    ))
}

fn systemd_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
pub fn systemd_unit(
    connection: &str,
    executable: &Path,
    home: &Path,
    path: &str,
) -> Result<String> {
    label(connection)?;
    // systemd expands $ in ExecStart even inside quotes; $$ is its literal escape.
    let executable = systemd_quote(&executable.to_string_lossy().replace('$', "$$"));
    Ok(format!(
        "[Unit]\nDescription=Litecom agent ({connection})\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart={executable} --connection {connection} worker\nEnvironment={}\nEnvironment={}\nRestart=on-failure\nRestartSec=10\nKillMode=control-group\nUMask=0077\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(&format!("LITECOM_HOME={}", home.display())),
        systemd_quote(&format!("PATH={path}"))
    ))
}

pub fn install(connection: &str) -> Result<()> {
    RunnerConfig::load(connection)?;
    let executable = install_binary()?;
    let home = config::home()?.canonicalize()?;
    let log = log_path(connection)?;
    config::private_dir(log.parent().context("Missing log folder")?)?;
    if !log.exists() {
        config::write_private(&log, b"")?;
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let path = std::env::var("PATH").unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        let destination = service_path(connection)?;
        let content = launch_agent(connection, &executable, &home, &log, &path)?;
        // The shared LaunchAgents directory is not ours to chmod.
        std::fs::create_dir_all(
            destination
                .parent()
                .context("Missing LaunchAgents folder")?,
        )?;
        write_service(&destination, content.as_bytes())?;
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{}", label(connection)?)])
            .output();
        checked(
            Command::new("launchctl")
                .arg("bootstrap")
                .arg(domain)
                .arg(destination),
        )?;
    }
    #[cfg(target_os = "linux")]
    {
        let destination = service_path(connection)?;
        std::fs::create_dir_all(destination.parent().context("Missing systemd folder")?)?;
        write_service(
            &destination,
            systemd_unit(connection, &executable, &home, &path)?.as_bytes(),
        )?;
        checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        checked(Command::new("systemctl").args([
            "--user",
            "enable",
            &format!("{}.service", label(connection)?),
        ]))?;
        // Restart also starts an inactive service. Using enable --now here
        // would start it twice and consume systemd's restart burst allowance.
        checked(Command::new("systemctl").args([
            "--user",
            "restart",
            &format!("{}.service", label(connection)?),
        ]))?;
    }
    #[cfg(target_os = "windows")]
    {
        checked(Command::new("powershell.exe").args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &windows_install_script(connection, &executable, &home)?,
        ]))?;
    }
    Ok(())
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_service(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing service folder")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
pub fn stop(connection: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    checked(Command::new("launchctl").args([
        "bootout",
        &format!("gui/{}/{}", unsafe { libc::getuid() }, label(connection)?),
    ]))?;
    #[cfg(target_os = "linux")]
    checked(Command::new("systemctl").args([
        "--user",
        "stop",
        &format!("{}.service", label(connection)?),
    ]))?;
    #[cfg(target_os = "windows")]
    checked(Command::new("powershell.exe").args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &windows_stop_script(connection)?,
    ]))?;
    Ok(())
}
pub fn remove(connection: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = stop(connection);
        let p = service_path(connection)?;
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .args([
                "--user",
                "disable",
                "--now",
                &format!("{}.service", label(connection)?),
            ])
            .output();
        let p = service_path(connection)?;
        if p.exists() {
            std::fs::remove_file(p)?;
        }
        checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    }
    #[cfg(target_os = "windows")]
    {
        stop(connection)?;
        checked(Command::new("powershell.exe").args([
            "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            &format!("$ErrorActionPreference = 'Stop'; {} | Unregister-ScheduledTask -Confirm:$false; $null = $true", windows_task_lookup(connection)?),
        ]))?;
    }
    Ok(())
}
/// A named service can belong to another --home. Never remove it by name alone.
pub(crate) fn remove_owned(connection: &str) -> Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let path = service_path(connection)?;
        if !path.try_exists()? {
            return Ok(());
        }
        let content = std::fs::read_to_string(&path)?;
        let home = config::home()?.canonicalize()?;
        #[cfg(target_os = "macos")]
        let owned = content.contains(&format!(
            "<key>LITECOM_HOME</key><string>{}</string>",
            xml(&home.to_string_lossy())
        ));
        #[cfg(target_os = "linux")]
        let owned = content.contains(&format!(
            "Environment={}",
            systemd_quote(&format!("LITECOM_HOME={}", home.display()))
        ));
        if !owned {
            bail!(
                "{} belongs to another Litecom installation. It was preserved.",
                path.display()
            );
        }
        #[cfg(target_os = "macos")]
        {
            // Missing jobs are fine, but failure to stop a running job is not.
            if status(connection)? {
                stop(connection)?;
            }
        }
        #[cfg(target_os = "linux")]
        checked(Command::new("systemctl").args([
            "--user",
            "disable",
            "--now",
            &format!("{}.service", label(connection)?),
        ]))?;
        remove(connection)?;
    }
    #[cfg(target_os = "windows")]
    {
        let executable = binary()?.to_string_lossy().replace('\'', "''");
        checked(Command::new("powershell.exe").args([
            "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            &format!("$ErrorActionPreference = 'Stop'; $task = {}; if ($task -and ($task.Actions.Execute -ne '{}')) {{ throw 'The startup task belongs to another Litecom installation.' }}; $null = $true", windows_task_lookup(connection)?, executable),
        ]))?;
        remove(connection)?;
    }
    Ok(())
}
pub fn status(connection: &str) -> Result<bool> {
    #[cfg(target_os = "macos")]
    let output = Command::new("launchctl")
        .args([
            "print",
            &format!("gui/{}/{}", unsafe { libc::getuid() }, label(connection)?),
        ])
        .output()?;
    #[cfg(target_os = "linux")]
    let output = Command::new("systemctl")
        .args([
            "--user",
            "is-active",
            &format!("{}.service", label(connection)?),
        ])
        .output()?;
    #[cfg(target_os = "windows")]
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &windows_status_script(connection)?,
        ])
        .output()?;
    #[cfg(target_os = "macos")]
    return Ok(output.status.success()
        && String::from_utf8_lossy(&output.stdout).contains("state = running"));
    #[cfg(target_os = "windows")]
    return Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "4");
    #[cfg(target_os = "linux")]
    Ok(output.status.success())
}

/// Update only services owned by this configuration directory. Never rewrite
/// their PATH, environment, startup policy, or working directory during upgrade.
pub(crate) fn running_owned(connection: &str) -> Result<bool> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let path = service_path(connection)?;
        if !path.try_exists()? {
            return Ok(false);
        }
        let content = std::fs::read_to_string(&path)?;
        let home = config::home()?.canonicalize()?;
        #[cfg(target_os = "macos")]
        let owned = content.contains(&format!(
            "<key>LITECOM_HOME</key><string>{}</string>",
            xml(&home.to_string_lossy())
        )) && content.contains(&format!(
            "<string>{}</string>",
            xml(&binary()?.to_string_lossy())
        ));
        #[cfg(target_os = "linux")]
        let owned = content.contains(&format!(
            "Environment={}",
            systemd_quote(&format!("LITECOM_HOME={}", home.display()))
        )) && content.contains(&format!(
            "ExecStart={} ",
            systemd_quote(&binary()?.to_string_lossy().replace('$', "$$"))
        ));
        if !owned {
            bail!(
                "The startup service for {connection} belongs to another installation. It was preserved."
            );
        }
    }
    #[cfg(target_os = "windows")]
    {
        let executable = binary()?.to_string_lossy().replace('\'', "''");
        let output = Command::new("powershell.exe").args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            &format!("$ErrorActionPreference='Stop'; $task = {}; if (-not $task) {{ exit 0 }}; if ($task.Actions.Execute -ne '{executable}') {{ throw 'The startup task belongs to another installation.' }}; Write-Output present", windows_task_lookup(connection)?)
        ]).output()?;
        if !output.status.success() {
            bail!("Cannot verify startup ownership for {connection}.");
        }
        if String::from_utf8_lossy(&output.stdout).trim().is_empty() {
            return Ok(false);
        }
    }
    status(connection)
}

pub(crate) fn start_existing(connection: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    checked(
        Command::new("launchctl")
            .arg("bootstrap")
            .arg(format!("gui/{}", unsafe { libc::getuid() }))
            .arg(service_path(connection)?),
    )?;
    #[cfg(target_os = "linux")]
    checked(Command::new("systemctl").args([
        "--user",
        "start",
        &format!("{}.service", label(connection)?),
    ]))?;
    #[cfg(target_os = "windows")]
    checked(Command::new("powershell.exe").args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "$ErrorActionPreference='Stop'; Start-ScheduledTask -TaskName '{}'",
            label(connection)?
        ),
    ]))?;
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn windows_install_script(connection: &str, executable: &Path, home: &Path) -> Result<String> {
    fn powershell_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
    // Quote the single home argument for Windows argv parsing, including a
    // trailing backslash. PowerShell quoting alone does not quote task arguments.
    fn argument(value: &str) -> String {
        let mut out = String::from("\"");
        let mut slashes = 0;
        for ch in value.chars() {
            if ch == '\\' {
                slashes += 1;
                continue;
            }
            out.push_str(&"\\".repeat(if ch == '"' { slashes * 2 + 1 } else { slashes }));
            slashes = 0;
            out.push(ch);
        }
        out.push_str(&"\\".repeat(slashes * 2));
        out.push('"');
        out
    }
    let name = powershell_literal(&label(connection)?);
    let executable = powershell_literal(&executable.to_string_lossy());
    // Task Scheduler does not retain the installing terminal's environment.
    // No credentials enter the task's arguments.
    let arguments = powershell_literal(&format!(
        "--home {} --connection {connection} worker",
        argument(&home.to_string_lossy())
    ));
    Ok(format!(
        "$ErrorActionPreference = 'Stop'; \
         $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name; \
         $action = New-ScheduledTaskAction -Execute {executable} -Argument {arguments}; \
         $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user; \
         $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited; \
         $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries \
             -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -StartWhenAvailable; \
         {stop} \
         Register-ScheduledTask -TaskName {name} -Action $action -Trigger $trigger \
             -Principal $principal -Settings $settings -Force | Out-Null; \
         Start-ScheduledTask -TaskName {name}",
        stop = windows_stop_script(connection)?
    ))
}

#[cfg(any(target_os = "windows", test))]
fn windows_task_lookup(connection: &str) -> Result<String> {
    // A named lookup emits a CIM error when the task is absent, even with
    // SilentlyContinue. Filtering a successful enumeration distinguishes absence
    // from permission/service failures and makes repeated removal succeed.
    Ok(format!(
        "Get-ScheduledTask -ErrorAction Stop | Where-Object {{ $_.TaskName -eq '{}' -and $_.TaskPath -eq '\\' }}",
        label(connection)?
    ))
}

#[cfg(any(target_os = "windows", test))]
fn windows_stop_script(connection: &str) -> Result<String> {
    // Task Scheduler can report Ready before its process exits. Hold handles to
    // this task's workers before stopping so restart/removal waits for release of
    // the executable and files, without affecting another Litecom connection.
    Ok(format!(
        "$ErrorActionPreference = 'Stop'; \
         $existing = {lookup}; \
         if ($existing) {{ \
             $workers = @(Get-CimInstance Win32_Process -Filter \"Name = 'litecom.exe'\" | \
                 Where-Object {{ $_.ExecutablePath -in $existing.Actions.Execute -and \
                     $_.CommandLine -like '*--connection {connection} worker' }} | \
                 ForEach-Object {{ \
                     $worker = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; \
                     if ($worker) {{ $null = $worker.Handle; $worker; }} \
                 }}); \
             Stop-ScheduledTask -InputObject $existing; \
             foreach ($worker in $workers) {{ \
                 if (-not $worker.WaitForExit(30000)) {{ throw 'The background worker has not stopped. Retry before updating or removing it.'; }} \
             }} \
         }}; $null = $true;",
        lookup = windows_task_lookup(connection)?
    ))
}

#[cfg(any(target_os = "windows", test))]
fn windows_status_script(connection: &str) -> Result<String> {
    // Task Scheduler's invariant numeric state distinguishes Running (4) from
    // Ready (3). schtasks /Query succeeds for both, and its labels are localized.
    Ok(format!(
        "$ErrorActionPreference = 'Stop'; $scheduler = New-Object -ComObject Schedule.Service; $scheduler.Connect(); [int]$scheduler.GetFolder('\\').GetTask('{}').State",
        label(connection)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeating_binary_install_preserves_the_running_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let installed = temp.path().join("bin/litecom");
        std::fs::write(&source, b"first binary").unwrap();
        assert!(copy_binary(&source, &installed).unwrap());
        let modified = std::fs::metadata(&installed).unwrap().modified().unwrap();
        #[cfg(windows)]
        let running = {
            use std::os::windows::fs::OpenOptionsExt;
            // A running Windows binary can be read, but cannot be replaced.
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&installed)
                .unwrap()
        };
        assert!(!copy_binary(&source, &installed).unwrap());
        assert_eq!(
            std::fs::metadata(&installed).unwrap().modified().unwrap(),
            modified
        );
        std::fs::write(&source, b"updated binary").unwrap();
        #[cfg(windows)]
        {
            assert!(copy_binary(&source, &installed).is_err());
            assert_eq!(std::fs::read(&installed).unwrap(), b"first binary");
            drop(running);
        }
        assert!(copy_binary(&source, &installed).unwrap());
        assert_eq!(std::fs::read(&installed).unwrap(), b"updated binary");
    }

    #[test]
    fn windows_status_uses_validated_names_and_numeric_task_state() {
        let script = windows_status_script("qa-status").unwrap();
        assert!(script.contains("GetTask('org.litecom.qa-status').State"));
        assert!(script.contains("[int]"));
        assert!(windows_status_script("bad'; Write-Output '4").is_err());
    }

    #[test]
    fn windows_startup_keeps_paths_literal_without_runtime_or_battery_restrictions() {
        let script = windows_install_script(
            "qa-startup",
            Path::new(r"C:\Users\O'Brien\$agent\litecom.exe"),
            Path::new(r"C:\Users\O'Brien\config folder\"),
        )
        .unwrap();
        assert!(script.contains(r"-Execute 'C:\Users\O''Brien\$agent\litecom.exe'"));
        assert!(script.contains(
            r#"--home "C:\Users\O''Brien\config folder\\" --connection qa-startup worker"#
        ));
        assert!(script.contains("-ExecutionTimeLimit ([TimeSpan]::Zero)"));
        assert!(script.contains("-AllowStartIfOnBatteries -DontStopIfGoingOnBatteries"));
        assert!(script.contains("-LogonType Interactive -RunLevel Limited"));
        assert!(windows_install_script("bad';", Path::new("x"), Path::new("y")).is_err());
    }
}
