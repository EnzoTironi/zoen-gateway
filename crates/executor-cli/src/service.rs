//! OS service unit writers: systemd --user, launchd, schtasks.

use std::path::{Path, PathBuf};
use std::process::Command;

use executor_host::DEFAULT_SERVICE_PORT;
use executor_sdk::data_dir;

/// `sh.executor.daemon` — original service label.
#[allow(dead_code)] // used on macOS/Windows backends and in tests
pub const SERVICE_LABEL: &str = "sh.executor.daemon";

/// Write the platform unit and print enable instructions.
///
/// # Errors
///
/// IO.
pub fn install(
    dir: Option<&Path>,
    boot: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exe = std::env::current_exe()?;
    let data = data_dir(dir);
    #[cfg(target_os = "macos")]
    {
        return install_launchd(&exe, &data, boot);
    }
    #[cfg(target_os = "windows")]
    {
        return install_schtasks(&exe, &data, boot);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        install_systemd(&exe, &data, boot)
    }
}

/// Remove the platform unit.
///
/// # Errors
///
/// IO.
pub fn uninstall() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "macos")]
    {
        return uninstall_launchd();
    }
    #[cfg(target_os = "windows")]
    {
        return uninstall_schtasks();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        uninstall_systemd()
    }
}

fn install_systemd(
    exe: &Path,
    data: &Path,
    boot: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let unit = format!(
        "[Unit]\nDescription=Executor daemon\n[Service]\nExecStart={} daemon run --foreground --port {DEFAULT_SERVICE_PORT} --hostname 127.0.0.1\nEnvironment=EXECUTOR_DATA_DIR={}\nRestart=on-failure\n[Install]\nWantedBy=default.target\n",
        exe.display(),
        data.display()
    );
    let path = systemd_unit_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, unit)?;
    println!("wrote {}", path.display());
    if boot {
        let _ = Command::new("loginctl").args(["enable-linger"]).status();
        println!("boot: lingering enabled (best-effort)");
    }
    println!("enable with: systemctl --user enable --now executor.service");
    Ok(())
}

fn uninstall_systemd() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = systemd_unit_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("not installed"),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn systemd_unit_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join(".config/systemd/user/executor.service"))
}

#[cfg(target_os = "macos")]
fn install_launchd(
    exe: &Path,
    data: &Path,
    _boot: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>daemon</string>
    <string>run</string>
    <string>--foreground</string>
    <string>--port</string>
    <string>{DEFAULT_SERVICE_PORT}</string>
    <string>--hostname</string>
    <string>127.0.0.1</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>EXECUTOR_DATA_DIR</key><string>{}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict></plist>
"#,
        exe.display(),
        data.display()
    );
    let path = launchd_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, plist)?;
    println!("wrote {}", path.display());
    println!("load with: launchctl load {}", path.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_launchd() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = launchd_path()?;
    let _ = Command::new("launchctl")
        .args(["unload", &path.to_string_lossy()])
        .status();
    match std::fs::remove_file(&path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("not installed"),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn launchd_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

#[cfg(target_os = "windows")]
fn install_schtasks(
    exe: &Path,
    data: &Path,
    boot: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let trigger = if boot { "ONSTART" } else { "ONLOGON" };
    let args =
        format!("daemon run --foreground --port {DEFAULT_SERVICE_PORT} --hostname 127.0.0.1");
    let status = Command::new("schtasks")
        .args([
            "/Create",
            "/TN",
            SERVICE_LABEL,
            "/SC",
            trigger,
            "/TR",
            &format!("\"{}\" {args}", exe.display()),
            "/F",
        ])
        .env("EXECUTOR_DATA_DIR", data)
        .status()?;
    if status.success() {
        println!("registered scheduled task {SERVICE_LABEL} ({trigger})");
    } else {
        return Err("schtasks /Create failed".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn uninstall_schtasks() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", SERVICE_LABEL, "/F"])
        .status();
    println!("removed scheduled task {SERVICE_LABEL}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SERVICE_LABEL;

    #[test]
    fn label_matches_original() {
        assert_eq!(SERVICE_LABEL, "sh.executor.daemon");
    }
}
