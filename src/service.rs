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
fn binary() -> Result<PathBuf> {
    Ok(config::home()?.join("bin").join(if cfg!(windows) {
        "litecom.exe"
    } else {
        "litecom"
    }))
}
pub fn install_binary() -> Result<PathBuf> {
    let dest = binary()?;
    let source = std::env::current_exe()?;
    if source != dest {
        config::write_private(&dest, &std::fs::read(source)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(dest)
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

fn xml(value: &str) -> String {
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
            "--now",
            &format!("{}.service", label(connection)?),
        ]))?;
        checked(Command::new("systemctl").args([
            "--user",
            "restart",
            &format!("{}.service", label(connection)?),
        ]))?;
    }
    #[cfg(target_os = "windows")]
    {
        // Pass LITECOM_HOME as a CLI option: Task Scheduler does not retain the
        // installing terminal's environment. No credentials enter task arguments.
        let command = format!(
            "\"{}\" --home \"{}\" --connection {} worker",
            executable.display(),
            home.display(),
            connection
        );
        checked(Command::new("schtasks").args([
            "/Create",
            "/SC",
            "ONLOGON",
            "/TN",
            &label(connection)?,
            "/TR",
            &command,
            "/IT",
            "/F",
        ]))?;
        checked(Command::new("schtasks").args(["/Run", "/TN", &label(connection)?]))?;
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
    checked(Command::new("schtasks").args(["/End", "/TN", &label(connection)?]))?;
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
        let _ = stop(connection);
        checked(Command::new("schtasks").args(["/Delete", "/TN", &label(connection)?, "/F"]))?;
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
    let output = Command::new("schtasks")
        .args(["/Query", "/TN", &label(connection)?])
        .output()?;
    #[cfg(target_os = "macos")]
    return Ok(output.status.success()
        && String::from_utf8_lossy(&output.stdout).contains("state = running"));
    #[cfg(not(target_os = "macos"))]
    Ok(output.status.success())
}
