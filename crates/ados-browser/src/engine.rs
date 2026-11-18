//! Browser discovery and process isolation. No user browser profile is reused.
use crate::state::Engine;
use anyhow::{bail, Context, Result};
use rustix::process::{kill_process, kill_process_group, Pid, Signal};
use std::{
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

pub struct Process(pub Child, PathBuf);
impl Drop for Process {
    fn drop(&mut self) {
        let pid = Pid::from_child(&self.0);
        let _ = kill_process_group(pid, Signal::TERM);
        thread::sleep(Duration::from_millis(100));
        let _ = kill_process_group(pid, Signal::KILL);
        let _ = self.0.kill();
        let _ = self.0.wait();
        cleanup_crashpad(&self.1);
    }
}
// Crashpad daemonizes outside Chromium's process group. Only handlers whose
// database is INSIDE our dedicated browser directory may be terminated.
fn cleanup_crashpad(root: &Path) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    let prefix = format!("--database={}/", root.display()).into_bytes();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
            .and_then(Pid::from_raw)
        else {
            continue;
        };
        let Ok(bytes) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut args = bytes.split(|b| *b == 0);
        if !args
            .next()
            .is_some_and(|arg| arg.ends_with(b"/chrome_crashpad_handler"))
        {
            continue;
        }
        if args.any(|arg| arg.starts_with(&prefix)) {
            let _ = kill_process(pid, Signal::KILL);
        }
    }
}
/// A Chromium-family browser is installed (the engine an automatic switch would start).
pub fn chromium_found() -> bool { executable(Engine::Chromium).is_ok() }
fn executable(engine: Engine) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if engine == Engine::Servo {
        // ADOS_SERVO (a full path) wins, then ~/Apps/servo, then `servoshell` on PATH.
        let search = std::env::var_os("PATH").unwrap_or_default();
        let on_path = std::env::split_paths(&search).map(|dir| dir.join("servoshell"));
        for path in std::env::var_os("ADOS_SERVO")
            .map(PathBuf::from)
            .into_iter()
            .chain([
                home.join("Apps/servo/servo/servoshell"),
                home.join("Apps/servo/servoshell"),
                home.join("Apps/servo/servo"),
            ])
            .chain(on_path)
        {
            if path.is_file() {
                return Ok(path);
            }
        }
        bail!("Servo was not found (ADOS_SERVO, ~/Apps/servo, or servoshell on PATH); install it or switch to Chromium");
    }
    // Reuse CloakBrowser's installed Chromium, not its Python automation stack.
    for cache in [home.join(".cloakbrowser"), home.join(".cache/cloakbrowser")] {
        if let Ok(dirs) = std::fs::read_dir(cache) {
            let mut dirs: Vec<_> = dirs.flatten().map(|e| e.path()).collect();
            dirs.sort();
            dirs.reverse();
            for dir in dirs {
                let p = dir.join("chrome");
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for name in [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "vivaldi-stable",
        "vivaldi",
    ] {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    bail!("No Chromium-family browser found. Install Chromium or Vivaldi.");
}
pub fn launch(engine: Engine, url: &str, port: u16, marker: &str, hints: &serde_json::Value) -> Result<Process> {
    let binary = executable(engine)?;
    let data = ados_core::paths::home().join("browser");
    std::fs::create_dir_all(&data)?;
    // util-linux applies PR_SET_PDEATHSIG before a direct exec, without unsafe
    // pre_exec code. A SIGKILLed aDOs therefore cannot leave servoshell alive.
    let mut command = Command::new("/usr/bin/setpriv");
    command.args(["--pdeathsig", "KILL", "--"]).arg(&binary);
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    // Vendor crash reporters can ignore --user-data-dir. Isolate XDG as well.
    for (key, folder) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
    ] {
        let path = data.join(folder);
        std::fs::create_dir_all(&path)?;
        command.env(key, path);
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data.join("engine.log"))?;
    command.stderr(log);
    match engine {
        Engine::Servo => {
            std::fs::create_dir_all(data.join("servo-profile"))?;
            // Link hints on every page load. Without them the browser still works.
            if let Ok(dir) = crate::hints::userscripts(hints) {
                command.arg(format!("--userscripts={}", dir.display()));
            }
            command.args([
                format!("--webdriver={port}"),
                format!("--config-dir={}", data.join("servo-profile").display()),
                "--no-native-titlebar".into(),
                "--window-size=1024x768".into(),
                url.into(),
            ]);
        }
        Engine::Chromium => {
            command.args([
                format!("--app={url}"),
                "--ozone-platform=x11".into(),
                format!(
                    "--user-data-dir={}",
                    data.join("chromium-profile").display()
                ),
                format!("--remote-debugging-port={port}"),
                "--remote-debugging-address=127.0.0.1".into(),
                format!("--class={marker}"),
                "--no-first-run".into(),
                "--no-default-browser-check".into(),
                "--disable-session-crashed-bubble".into(),
                "--disable-dev-shm-usage".into(),
                "--disable-breakpad".into(),
                "--disable-crash-reporter".into(),
                "--disable-background-networking".into(),
                "--disable-component-update".into(),
            ]);
        }
    }
    Ok(Process(
        command
            .spawn()
            .with_context(|| format!("Could not start {}", binary.display()))?,
        data,
    ))
}
