use std::process::Command;

pub struct Imported {
    pub vars: Vec<(String, String)>,
    pub aliases: Vec<(String, String)>,
}

const MARK_ENV: &str = "===XSH_ENV_MARK===";
const MARK_ALIAS: &str = "===XSH_ALIAS_MARK===";

pub fn import(shell_bin: &str, rc_filename: &str) -> Option<Imported> {
    if which(shell_bin).is_none() {
        return None;
    }
    let home = dirs::home_dir()?;
    let rc_path = home.join(rc_filename);
    if !rc_path.exists() {
        return None;
    }
    let script = format!(
        "source '{}' >/dev/null 2>&1; echo {mark_env}; env; echo {mark_alias}; alias",
        rc_path.display(),
        mark_env = MARK_ENV,
        mark_alias = MARK_ALIAS,
    );
    let output = Command::new(shell_bin)
        .arg("-i")
        .arg("-c")
        .arg(&script)
        .env("XSH_IMPORTING", "1")
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    let idx_env = raw.find(MARK_ENV)?;
    let idx_alias = raw.find(MARK_ALIAS)?;
    let env_part = &raw[idx_env + MARK_ENV.len()..idx_alias];
    let alias_part = &raw[idx_alias + MARK_ALIAS.len()..];

    let mut vars = Vec::new();
    for line in env_part.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if !k.is_empty() {
                vars.push((k.to_string(), v.to_string()));
            }
        }
    }

    let mut aliases = Vec::new();
    for line in alias_part.lines() {
        let line = line.trim();
        let line = line.strip_prefix("alias ").unwrap_or(line);
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            let v = v
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .or_else(|| v.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
                .unwrap_or(v);
            aliases.push((k.to_string(), v.to_string()));
        }
    }

    Some(Imported { vars, aliases })
}

fn which(bin: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        let candidate = std::path::Path::new(dir).join(bin);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}
