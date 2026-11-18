//! Lighter launch for backends written as `npx -y pkg@ver`.
//!
//! `npx` keeps `npm exec` and a `sh` alive for the agent's whole life (85–108 MB). When the
//! package is already in npx's own cache, aDOs runs its bin directly (`node <bin> args…`) and
//! remembers the path in `ADOS_HOME/cache/agents/<pkg@ver>.path`. When it is not (first run),
//! the backend starts through `npx` as written, which fills npx's cache for next time.
use std::path::{Path, PathBuf};

use crate::config::{ados_home, BackendConfig};

/// The direct launch for `config`, if it is an `npx` backend whose package can be found.
pub fn resolve(config: &BackendConfig) -> Option<BackendConfig> {
    let (spec, rest) = npx_spec(config)?;
    let (name, version) = split_spec(&spec)?;
    let cache_file = ados_home().join("cache/agents").join(format!("{}.path", sanitize(&spec)));
    let bin = match std::fs::read_to_string(&cache_file) {
        Ok(text) if Path::new(text.trim()).is_file() => PathBuf::from(text.trim()),
        _ => {
            let bin = npx_caches().iter().find_map(|root| find_in_npx_cache(root, name, version))?;
            if let Some(dir) = cache_file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&cache_file, bin.display().to_string());
            bin
        }
    };
    let mut direct = config.clone();
    if runs_with_node(&bin) {
        direct.command = which("node")?.display().to_string();
        direct.args = std::iter::once(bin.display().to_string()).chain(rest).collect();
    } else {
        direct.command = bin.display().to_string();
        direct.args = rest;
    }
    Some(direct)
}

/// `npx -y pkg@ver a b` → (`pkg@ver`, [a, b]). Other npx flags: not handled, keep npx.
fn npx_spec(config: &BackendConfig) -> Option<(String, Vec<String>)> {
    // `file_stem`: Windows configs may say `npx.cmd`.
    let program = Path::new(&config.command).file_stem()?.to_str()?;
    if program != "npx" {
        return None;
    }
    let mut args = config.args.iter().skip_while(|a| *a == "-y" || *a == "--yes");
    let spec = args.next()?.clone();
    if spec.starts_with('-') {
        return None;
    }
    Some((spec, args.cloned().collect()))
}

/// `@scope/name@1.2.3` → (`@scope/name`, `1.2.3`). A spec without an exact version gives `None`.
fn split_spec(spec: &str) -> Option<(&str, &str)> {
    let at = spec.rfind('@').filter(|&i| i > 0)?;
    let (name, version) = (&spec[..at], &spec[at + 1..]);
    let exact = !version.is_empty() && version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    (exact && version.starts_with(|c: char| c.is_ascii_digit())).then_some((name, version))
}

fn sanitize(spec: &str) -> String {
    spec.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

/// npm's cache folders that may hold `_npx/`: `$npm_config_cache`, else `~/.npm`
/// (Windows: `%LOCALAPPDATA%\npm-cache`).
fn npx_caches() -> Vec<PathBuf> {
    let mut roots = vec![];
    if let Some(dir) = std::env::var_os("npm_config_cache") {
        roots.push(PathBuf::from(dir));
    }
    if let Some(home) = std::env::home_dir() {
        roots.push(home.join(".npm"));
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("npm-cache"));
    }
    roots
}

/// Look through `<root>/_npx/*/node_modules/<name>` for `version` and return its bin file.
fn find_in_npx_cache(root: &Path, name: &str, version: &str) -> Option<PathBuf> {
    std::fs::read_dir(root.join("_npx")).ok()?.flatten().find_map(|dir| {
        let pkg = dir.path().join("node_modules").join(name);
        bin_of(&pkg, name, version)
    })
}

/// The package's bin file, if `pkg/package.json` is `version` and names one that exists.
fn bin_of(pkg: &Path, name: &str, version: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(pkg.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if json["version"].as_str()? != version {
        return None;
    }
    let base = name.rsplit('/').next().unwrap_or(name);
    let rel = match &json["bin"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            let pick = map.get(base).or_else(|| (map.len() == 1).then(|| map.values().next()).flatten())?;
            pick.as_str()?.to_string()
        }
        _ => return None,
    };
    let bin = pkg.join(rel);
    bin.is_file().then_some(bin)
}

/// A `.js`/`.mjs`/`.cjs` file or a `#!…node` script runs under node; anything else runs itself.
fn runs_with_node(bin: &Path) -> bool {
    if matches!(bin.extension().and_then(|e| e.to_str()), Some("js" | "mjs" | "cjs")) {
        return true;
    }
    let mut head = [0u8; 128];
    let n = std::fs::File::open(bin).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..n]);
    head.starts_with("#!") && head.lines().next().is_some_and(|l| l.contains("node"))
}

/// The program to start. Windows: npm installs `npx` and agent commands as `.cmd` files, and
/// `Command` only appends `.exe` to a bare name, so a bare name is resolved on `PATH` here.
pub fn program(command: &str) -> String {
    if cfg!(windows) && !command.contains(['/', '\\']) {
        if let Some(path) = which(command) {
            return path.display().to_string();
        }
    }
    command.to_string()
}

fn which(program: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) && Path::new(program).extension().is_none() {
        vec![format!("{program}.exe"), format!("{program}.cmd")]
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npx(args: &[&str]) -> BackendConfig {
        BackendConfig {
            id: "x".into(),
            name: "X".into(),
            command: "npx".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Default::default(),
        }
    }

    #[test]
    fn reads_npx_specs() {
        assert_eq!(npx_spec(&npx(&["-y", "pi-acp@0.0.34"])), Some(("pi-acp@0.0.34".into(), vec![])));
        assert_eq!(npx_spec(&npx(&["--yes", "a@1.0.0", "--flag"])), Some(("a@1.0.0".into(), vec!["--flag".into()])));
        assert_eq!(npx_spec(&npx(&["-p", "a@1", "b"])), None);
        let mut other = npx(&["-y", "a@1.0.0"]);
        other.command = "claude-agent-acp".into();
        assert_eq!(npx_spec(&other), None);
        assert_eq!(split_spec("@agentclientprotocol/codex-acp@2.6.0"), Some(("@agentclientprotocol/codex-acp", "2.6.0")));
        assert_eq!(split_spec("pi-acp@0.0.34"), Some(("pi-acp", "0.0.34")));
        assert_eq!(split_spec("pi-acp"), None);
        assert_eq!(split_spec("pi-acp@latest"), None);
        assert_eq!(split_spec("@scope/name"), None);
    }

    #[test]
    fn finds_the_bin_in_an_npx_cache() {
        let root = std::env::temp_dir().join(format!("ados-launch-test-{}", std::process::id()));
        let pkg = root.join("_npx/abc123/node_modules/@scope/tool-acp");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(pkg.join("dist/index.js"), "#!/usr/bin/env node\n").unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"@scope/tool-acp","version":"2.6.0","bin":{"tool-acp":"dist/index.js","other":"x.js"}}"#,
        )
        .unwrap();
        assert_eq!(find_in_npx_cache(&root, "@scope/tool-acp", "2.6.0"), Some(pkg.join("dist/index.js")));
        assert_eq!(find_in_npx_cache(&root, "@scope/tool-acp", "2.7.0"), None, "other version");
        assert!(runs_with_node(&pkg.join("dist/index.js")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
