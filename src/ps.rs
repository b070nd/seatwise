//! `seat ps`: the Claude Code sessions running right now, and which seat each one is on.

use crate::config::{Config, Seat};
use crate::folders::key_string;
use crate::paths;
use std::path::PathBuf;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub struct Session {
    pub pid: u32,
    /// The seat, or how the config folder was found when it isn't a configured seat.
    pub seat: String,
    pub cwd: Option<PathBuf>,
    pub minutes: u64,
    /// `preset@fingerprint` when seatwise started it.
    pub preset: Option<String>,
}

/// A Claude Code CLI process: the native `claude` binary, or Node running the `claude-code` package.
fn is_claude_code(name: &str, cmd: &[String]) -> bool {
    let name = name.to_ascii_lowercase();
    let stem = name.trim_end_matches(".exe");
    stem == "claude"
        || stem == "codex"
        || ((stem == "node" || stem == "bun")
            && cmd.iter().any(|a| {
                a.contains("claude-code") || a.ends_with("/claude") || a.contains("@openai/codex")
            }))
}

fn seat_for<'a>(cfg: &'a Config, env: &[(String, String)]) -> Result<&'a Seat, String> {
    let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if let Some(s) = get("SEATWISE_SEAT").and_then(|s| cfg.seat(s)) {
        return Ok(s);
    }
    let codex = get("CODEX_HOME").is_some() && get("CLAUDE_CONFIG_DIR").is_none();
    let dir = get("CLAUDE_CONFIG_DIR")
        .or_else(|| get("CODEX_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(if codex { ".codex" } else { ".claude" }));
    let key = key_string(&dir);
    cfg.seats
        .iter()
        .find(|s| key_string(&s.config_dir()) == key)
        .ok_or_else(|| format!("({})", paths::contract(&dir)))
}

pub fn sessions(cfg: &Config) -> anyhow::Result<Vec<Session>> {
    let mut sys = System::new().map_err(|e| anyhow::anyhow!("reading the process list: {e:?}"))?;
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_environ(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );
    let claude: std::collections::HashSet<Pid> = sys
        .processes()
        .iter()
        .filter(|(_, p)| {
            let cmd: Vec<String> = p
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            is_claude_code(&p.name().to_string_lossy(), &cmd)
        })
        .map(|(pid, _)| *pid)
        .collect();
    let mut out: Vec<Session> = claude
        .iter()
        .filter_map(|pid| {
            let p = sys.process(*pid)?;
            // A child of another Claude Code process is part of that session.
            if p.parent().is_some_and(|pp| claude.contains(&pp)) {
                return None;
            }
            let env: Vec<(String, String)> = p
                .environ()
                .iter()
                .filter_map(|kv| {
                    let kv = kv.to_string_lossy();
                    kv.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                })
                .collect();
            Some(Session {
                pid: pid.as_u32(),
                seat: match seat_for(cfg, &env) {
                    Ok(s) => s.name.clone(),
                    Err(dir) => dir,
                },
                cwd: p.cwd().map(PathBuf::from),
                minutes: p.run_time() / 60,
                preset: env
                    .iter()
                    .find(|(k, _)| k == "SEATWISE_PRESET")
                    .map(|(_, v)| v.clone()),
            })
        })
        .collect();
    out.sort_by(|a, b| a.seat.cmp(&b.seat).then(a.pid.cmp(&b.pid)));
    Ok(out)
}

pub fn list(cfg: &Config) -> anyhow::Result<String> {
    let sessions = sessions(cfg)?;
    if sessions.is_empty() {
        return Ok("no Claude Code sessions running\n".into());
    }
    let mut rows = vec![
        ["SEAT", "PID", "AGE", "PRESET", "FOLDER"]
            .map(String::from)
            .to_vec(),
    ];
    for s in sessions {
        rows.push(vec![
            s.seat,
            s.pid.to_string(),
            format!("{}m", s.minutes),
            s.preset.unwrap_or_else(|| "-".into()),
            s.cwd
                .map(|c| paths::contract(&c))
                .unwrap_or_else(|| "?".into()),
        ]);
    }
    Ok(crate::status::table(&rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_claude_code() {
        let cmd = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_claude_code("claude", &[]));
        assert!(is_claude_code("claude.exe", &[]));
        assert!(is_claude_code("codex", &[]));
        assert!(is_claude_code(
            "node",
            &cmd(&[
                "node",
                "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"
            ])
        ));
        assert!(!is_claude_code("node", &cmd(&["node", "server.js"])));
        assert!(
            !is_claude_code("Claude Helper", &[]),
            "desktop app helpers are not CLI sessions"
        );
    }
}
