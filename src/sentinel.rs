//! The sentinel: fixed, explainable checks for settings that cost more than they need to. It only reports; it never
//! changes anything. The same files give the same findings, each with its rule, its scope (a seat, or one project),
//! the evidence and the change that would fix it. Rules can be silenced (`sentinel.ignore`).
//!
//! It runs when a seat starts (at most once per `sentinel.interval`, when `sentinel.enabled`), on demand with
//! `seat sentinel`, and on a schedule if you install one (`seat sentinel install`: launchd, systemd or Task Scheduler).

use crate::config::{Agent, Apply, Config, Seat, parse_interval};
use crate::errln;
use crate::folders::key_string;
use crate::plan::Plan;
use crate::{paths, preset};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Instruction files over this many characters are flagged (they are loaded into every session).
pub const INSTRUCTIONS_LIMIT: usize = 40_000;
/// Allow lists longer than this are flagged.
pub const ALLOW_LIMIT: usize = 40;
/// How many of a project's newest sessions are read to see which MCP servers are used.
const RECENT_SESSIONS: usize = 20;
/// MCP servers are only judged unused after at least this many sessions.
const MIN_SESSIONS: usize = 3;

pub const RULES: [(&str, &str); 5] = [
    (
        "plan-fit",
        "a seat's settings or a group's preset uses Opus on a plan with small limits",
    ),
    (
        "instructions-size",
        "CLAUDE.md files large enough to cost context on every turn",
    ),
    (
        "mcp-unused",
        "project MCP servers no recent session used (each one adds tools to every prompt)",
    ),
    (
        "allow-list",
        "permission allow lists that are long or repeat rules",
    ),
    (
        "preset-drift",
        "project settings that differ from the folder group's preset",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Seat(String),
    Project(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub scope: Scope,
    pub message: String,
    pub fix: String,
}

impl Finding {
    pub fn line(&self) -> String {
        let scope = match &self.scope {
            Scope::Seat(s) => format!("seat {s}"),
            Scope::Project(p) => paths::contract(p),
        };
        format!(
            "[{}] {scope}: {}\n    → {}",
            self.rule, self.message, self.fix
        )
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Opus as the main model (not `opusplan`, which only plans with it).
fn heavy(settings: &Value) -> bool {
    settings["model"]
        .as_str()
        .is_some_and(|m| m.contains("opus") && m != "opusplan")
}

fn small_plan(seat: &Seat) -> Option<Plan> {
    let plan = crate::claude::read_account(seat)
        .as_ref()
        .and_then(Plan::of)?;
    matches!(plan, Plan::Pro | Plan::TeamStandard).then_some(plan)
}

/// Seat-wide checks.
pub fn check_seat(cfg: &Config, seat: &Seat) -> Vec<Finding> {
    let mut out = Vec::new();
    if seat.agent != Agent::Claude {
        return out;
    }
    let n = &seat.name;
    let user = read_json(&seat.config_dir().join("settings.json")).unwrap_or_default();
    if let Some(plan) = small_plan(seat) {
        if heavy(&user) {
            out.push(Finding {
                rule: "plan-fit",
                scope: Scope::Seat(n.clone()),
                message: format!(
                    "settings.json makes {} the main model on a {} plan, which uses the limits up fastest",
                    user["model"].as_str().unwrap_or_default(),
                    plan.label()
                ),
                fix: format!("seat profile {n} light (or balanced)"),
            });
        }
        for g in cfg.groups.iter().filter(|g| g.enabled) {
            let owned = g.paths.iter().any(|p| {
                crate::folders::owner_of(
                    cfg,
                    &crate::folders::normalize(&paths::expand(p), &paths::home()),
                ) == Some(crate::config::Owner::Seat(&seat.name))
            });
            if let Some(p) = preset::get(cfg, &g.preset).filter(|p| p.enabled)
                && owned
                && heavy(&preset::settings(cfg, &p))
            {
                out.push(Finding {
                    rule: "plan-fit",
                    scope: Scope::Seat(n.clone()),
                    message: format!(
                        "group '{}' uses preset '{}' (Opus) in this seat's folders on a {} plan",
                        g.name,
                        p.name,
                        plan.label()
                    ),
                    fix: format!("give group '{}' a lighter preset (explore, quick)", g.name),
                });
            }
        }
    }
    out.extend(allow_list(
        &user,
        Scope::Seat(n.clone()),
        &paths::contract(&seat.config_dir().join("settings.json")),
    ));
    out
}

fn allow_list(settings: &Value, scope: Scope, file: &str) -> Vec<Finding> {
    let Some(allow) = settings["permissions"]["allow"].as_array() else {
        return vec![];
    };
    let mut seen = std::collections::HashSet::new();
    let dups = allow.iter().filter(|r| !seen.insert(r.to_string())).count();
    let mut out = Vec::new();
    if dups > 0 {
        out.push(Finding {
            rule: "allow-list",
            scope: scope.clone(),
            message: format!("{file} repeats {dups} permission rule(s)"),
            fix: "remove the repeated entries from permissions.allow".into(),
        });
    }
    if allow.len() > ALLOW_LIMIT {
        let exact = allow
            .iter()
            .filter_map(Value::as_str)
            .filter(|r| r.starts_with("Bash(") && !r.contains('*'))
            .count();
        out.push(Finding {
            rule: "allow-list",
            scope,
            message: format!(
                "{file} allows {} rules ({exact} exact Bash commands), usually one-off answers that piled up",
                allow.len()
            ),
            fix: "replace the exact commands with a few prefix rules, e.g. Bash(cargo test:*)".into(),
        });
    }
    out
}

/// How Claude Code names a project's session folder: every character other than a letter or digit becomes '-'.
pub fn sessions_dir(seat: &Seat, project: &Path) -> PathBuf {
    let name: String = project
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    seat.config_dir().join("projects").join(name)
}

/// The text of a project's newest sessions (bounded).
fn recent_sessions(seat: &Seat, project: &Path) -> Vec<String> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> =
        std::fs::read_dir(sessions_dir(seat, project))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    files
        .into_iter()
        .take(RECENT_SESSIONS)
        .filter_map(|(_, p)| {
            use std::io::Read;
            let mut s = String::new();
            std::fs::File::open(p)
                .ok()?
                .take(8 << 20)
                .read_to_string(&mut s)
                .ok()?;
            Some(s)
        })
        .collect()
}

/// Checks for one project, as used by `seat`.
pub fn check_project(cfg: &Config, seat: &Seat, root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    if seat.agent != Agent::Claude {
        return out;
    }
    let scope = || Scope::Project(root.to_path_buf());

    let size: usize = ["CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(root.join(f)).ok())
        .map(|t| t.chars().count())
        .sum();
    if size > INSTRUCTIONS_LIMIT {
        out.push(Finding {
            rule: "instructions-size",
            scope: scope(),
            message: format!(
                "CLAUDE.md files hold {size} characters, loaded into every session here (limit {INSTRUCTIONS_LIMIT})"
            ),
            fix: "move reference material into docs/ or a skill, and keep CLAUDE.md to what every task needs".into(),
        });
    }

    let local = read_json(&root.join(".claude/settings.local.json")).unwrap_or_default();
    let shared = read_json(&root.join(".claude/settings.json")).unwrap_or_default();
    if let Some(servers) = read_json(&root.join(".mcp.json")).and_then(|v| {
        v["mcpServers"]
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
    }) {
        let disabled: Vec<&str> = local["disabledMcpjsonServers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let sessions = recent_sessions(seat, root);
        if sessions.len() >= MIN_SESSIONS {
            let unused: Vec<&String> = servers
                .iter()
                .filter(|s| !disabled.contains(&s.as_str()))
                .filter(|s| {
                    let mark = format!("mcp__{s}__");
                    !sessions.iter().any(|t| t.contains(&mark))
                })
                .collect();
            if !unused.is_empty() {
                let names: Vec<&str> = unused.iter().map(|s| s.as_str()).collect();
                out.push(Finding {
                    rule: "mcp-unused",
                    scope: scope(),
                    message: format!(
                        "MCP server(s) {} were not used in the last {} sessions of seat '{}'",
                        names.join(", "),
                        sessions.len(),
                        seat.name
                    ),
                    fix: format!(
                        "add {:?} to disabledMcpjsonServers in .claude/settings.local.json",
                        names
                    ),
                });
            }
        }
    }

    out.extend(allow_list(&local, scope(), ".claude/settings.local.json"));

    let r = preset::resolve(cfg, seat, root);
    if let Some(name) = &r.preset {
        match r.apply {
            Apply::Project if !preset::contained(&local, &r.settings) => out.push(Finding {
                rule: "preset-drift",
                scope: scope(),
                message: format!(
                    "settings.local.json no longer matches preset '{name}' of group '{}'",
                    r.group.as_deref().unwrap_or_default()
                ),
                fix: format!(
                    "the next `seat run {}` here writes it back; or change the group's preset",
                    seat.name
                ),
            }),
            Apply::Session => {
                let mut on_disk = shared.clone();
                preset::merge_deep(&mut on_disk, &local);
                let differ: Vec<String> = preset::effective(&[preset::Layer {
                    name: String::new(),
                    path: None,
                    settings: r.settings.clone(),
                }])
                .into_iter()
                .filter(|(k, v, _)| {
                    let disk = k.split('.').fold(&on_disk, |v, part| &v[part]);
                    !disk.is_null() && disk != v
                })
                .map(|(k, _, _)| k)
                .collect();
                if !differ.is_empty() {
                    out.push(Finding {
                        rule: "preset-drift",
                        scope: scope(),
                        message: format!(
                            "the project's own settings set {} differently from preset '{name}': a plain `claude` here runs another way than `seat run`",
                            differ.join(", ")
                        ),
                        fix: format!(
                            "remove them from the project's settings, or set apply = \"project\" on group '{}'",
                            r.group.as_deref().unwrap_or_default()
                        ),
                    });
                }
            }
            Apply::Project => {}
        }
    }
    out
}

/// Projects a seat works in: its git repos, and its groups' projects.
pub fn projects(cfg: &Config, seat: &Seat) -> Vec<PathBuf> {
    let mut out = crate::home::repos(cfg, seat);
    if let Some(h) = seat.home()
        && h.is_dir()
    {
        out.push(h);
    }
    out.sort();
    out.dedup();
    out
}

fn ignored(cfg: &Config, f: &Finding) -> bool {
    cfg.sentinel.ignore.iter().any(|i| match i.split_once('@') {
        None => i == f.rule,
        Some((rule, path)) => {
            rule == f.rule
                && match &f.scope {
                    Scope::Project(p) => key_string(&paths::expand(path)) == key_string(p),
                    Scope::Seat(s) => s == path,
                }
        }
    })
}

/// Every check for the enabled seats (`only`: just these seats).
pub fn check_all(cfg: &Config, only: &[&Seat]) -> Vec<Finding> {
    let mut out = Vec::new();
    for seat in cfg.seats.iter().filter(|s| cfg.is_enabled(s)) {
        if !only.is_empty() && !only.iter().any(|o| o.name == seat.name) {
            continue;
        }
        out.extend(check_seat(cfg, seat));
        for p in projects(cfg, seat) {
            out.extend(check_project(cfg, seat, &p));
        }
    }
    out.retain(|f| !ignored(cfg, f));
    out.dedup();
    out
}

/// One seat in one folder: what a start checks.
pub fn check_here(cfg: &Config, seat: &Seat, dir: &Path) -> Vec<Finding> {
    let mut out = check_seat(cfg, seat);
    out.extend(check_project(cfg, seat, &preset::project_root(dir)));
    out.retain(|f| !ignored(cfg, f));
    out
}

fn dir() -> PathBuf {
    paths::data_dir().join("sentinel")
}

/// The last report, for `seat tui`.
pub fn last_report() -> Option<String> {
    std::fs::read_to_string(dir().join("report.txt")).ok()
}

fn save_report(findings: &[Finding]) {
    let _ = std::fs::create_dir_all(dir());
    let text: String = findings.iter().map(|f| f.line() + "\n").collect();
    let _ = std::fs::write(dir().join("report.txt"), text);
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether a check is due, and if so, mark it done now.
fn due(cfg: &Config) -> bool {
    let every = parse_interval(&cfg.sentinel.interval).unwrap_or(86400);
    let stamp = dir().join("last");
    let last: u64 = std::fs::read_to_string(&stamp)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if now() < last + every {
        return false;
    }
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(&stamp, now().to_string());
    true
}

pub fn notify(findings: &[Finding]) {
    if findings.is_empty() {
        return;
    }
    let body = findings
        .iter()
        .take(3)
        .map(|f| format!("{}: {}", f.rule, f.message))
        .collect::<Vec<_>>()
        .join("\n");
    let _ = notify_rust::Notification::new()
        .summary(&format!("seatwise: {} suggestion(s)", findings.len()))
        .body(&body)
        .show();
}

/// On `seat run`: check this seat and project when enabled and due. Never fails the start.
pub fn on_start(cfg: &Config, seat: &Seat, cwd: &Path) {
    if !cfg.sentinel.enabled || !due(cfg) {
        return;
    }
    let findings = check_here(cfg, seat, cwd);
    save_report(&findings);
    if findings.is_empty() {
        return;
    }
    errln!("seat: sentinel: {} suggestion(s):", findings.len());
    for f in findings.iter().take(5) {
        errln!("  {}", f.line().replace('\n', "\n  "));
    }
    errln!("  (all: `seat sentinel`; silence one with sentinel.ignore)");
    if cfg.sentinel.notify {
        notify(&findings);
    }
}

/// `seat sentinel`: run now; the report is kept for `seat tui`.
pub fn run(cfg: &Config, only: &[&Seat]) -> Vec<Finding> {
    let findings = check_all(cfg, only);
    save_report(&findings);
    findings
}

// ---- scheduler ----------------------------------------------------------------------------------------------

const LABEL: &str = "io.github.seatwise.sentinel";

/// What `seat sentinel install` writes and runs, so `--dry-run` can show it.
pub struct Schedule {
    pub files: Vec<(PathBuf, String)>,
    pub commands: Vec<Vec<String>>,
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Variables passed to the scheduled run, so it reads the same config as you do.
fn passed_env() -> Vec<(String, String)> {
    ["SEATWISE_CONFIG", "SEATWISE_DATA", "SEATWISE_HOME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

pub fn schedule(cfg: &Config, exe: &Path) -> Result<Schedule> {
    let secs = parse_interval(&cfg.sentinel.interval).context("bad sentinel.interval")?;
    let exe_s = exe.display().to_string();
    let home = paths::home();
    if cfg!(target_os = "macos") {
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        let env: String = passed_env()
            .iter()
            .map(|(k, v)| format!("    <key>{}</key><string>{}</string>\n", xml(k), xml(v)))
            .collect();
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \x20 <key>Label</key><string>{LABEL}</string>\n\
             \x20 <key>ProgramArguments</key>\n  <array><string>{}</string><string>sentinel</string><string>--notify</string><string>--quiet</string></array>\n\
             \x20 <key>StartInterval</key><integer>{secs}</integer>\n\
             \x20 <key>EnvironmentVariables</key>\n  <dict>\n{env}  </dict>\n\
             </dict>\n</plist>\n",
            xml(&exe_s)
        );
        let domain = format!("gui/{}", uid());
        return Ok(Schedule {
            commands: vec![
                vec![
                    "launchctl".into(),
                    "bootout".into(),
                    domain.clone(),
                    plist.display().to_string(),
                ],
                vec![
                    "launchctl".into(),
                    "bootstrap".into(),
                    domain,
                    plist.display().to_string(),
                ],
            ],
            files: vec![(plist, text)],
        });
    }
    if cfg!(windows) {
        let minutes = (secs / 60).max(1);
        let (sc, mo) = if minutes < 1440 {
            ("MINUTE", minutes)
        } else {
            ("DAILY", minutes / 1440)
        };
        return Ok(Schedule {
            files: vec![],
            commands: vec![vec![
                "schtasks".into(),
                "/Create".into(),
                "/F".into(),
                "/SC".into(),
                sc.into(),
                "/MO".into(),
                mo.to_string(),
                "/TN".into(),
                "seatwise-sentinel".into(),
                "/TR".into(),
                format!("\"{exe_s}\" sentinel --notify --quiet"),
            ]],
        });
    }
    let dir = home.join(".config/systemd/user");
    let env: String = passed_env()
        .iter()
        .map(|(k, v)| format!("Environment=\"{k}={v}\"\n"))
        .collect();
    Ok(Schedule {
        files: vec![
            (
                dir.join("seatwise-sentinel.service"),
                format!(
                    "[Unit]\nDescription=seatwise sentinel\n\n[Service]\nType=oneshot\n{env}ExecStart=\"{exe_s}\" sentinel --notify --quiet\n"
                ),
            ),
            (
                dir.join("seatwise-sentinel.timer"),
                format!(
                    "[Unit]\nDescription=seatwise sentinel every {}\n\n[Timer]\nOnBootSec=5min\nOnUnitActiveSec={secs}s\n\n[Install]\nWantedBy=timers.target\n",
                    cfg.sentinel.interval
                ),
            ),
        ],
        commands: vec![
            vec!["systemctl".into(), "--user".into(), "daemon-reload".into()],
            vec![
                "systemctl".into(),
                "--user".into(),
                "enable".into(),
                "--now".into(),
                "seatwise-sentinel.timer".into(),
            ],
        ],
    })
}

/// The reverse of `schedule`.
pub fn unschedule() -> Schedule {
    let home = paths::home();
    if cfg!(target_os = "macos") {
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        return Schedule {
            commands: vec![vec![
                "launchctl".into(),
                "bootout".into(),
                format!("gui/{}", uid()),
                plist.display().to_string(),
            ]],
            files: vec![(plist, String::new())],
        };
    }
    if cfg!(windows) {
        return Schedule {
            files: vec![],
            commands: vec![vec![
                "schtasks".into(),
                "/Delete".into(),
                "/F".into(),
                "/TN".into(),
                "seatwise-sentinel".into(),
            ]],
        };
    }
    let dir = home.join(".config/systemd/user");
    Schedule {
        commands: vec![vec![
            "systemctl".into(),
            "--user".into(),
            "disable".into(),
            "--now".into(),
            "seatwise-sentinel.timer".into(),
        ]],
        files: vec![
            (dir.join("seatwise-sentinel.service"), String::new()),
            (dir.join("seatwise-sentinel.timer"), String::new()),
        ],
    }
}

fn uid() -> String {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "501".into())
}

/// Write the files (empty text: remove) and run the commands. The first command of an install (unloading an old
/// job) may fail; the rest may not.
pub fn carry_out(s: &Schedule, install: bool) -> Result<()> {
    for (path, text) in &s.files {
        if text.is_empty() {
            let _ = std::fs::remove_file(path);
        } else {
            std::fs::create_dir_all(path.parent().expect("has parent"))?;
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    for (i, argv) in s.commands.iter().enumerate() {
        let ok = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|st| st.success());
        let may_fail = (install && i == 0 && s.commands.len() > 1) || !install;
        if !ok && !may_fail {
            anyhow::bail!("`{}` failed", argv.join(" "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allow_list_flags_repeats_and_piles() {
        let mut allow: Vec<String> = (0..45).map(|i| format!("Bash(echo {i})")).collect();
        allow.push("Bash(echo 1)".into());
        let f = allow_list(
            &json!({ "permissions": { "allow": allow } }),
            Scope::Seat("w".into()),
            "x",
        );
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(
            f[1].message.contains("46 rules (46 exact"),
            "{}",
            f[1].message
        );
        assert!(allow_list(&json!({}), Scope::Seat("w".into()), "x").is_empty());
    }

    #[test]
    fn sessions_dir_follows_claude_code() {
        let seat = Seat {
            name: "w".into(),
            config_dir: "/c".into(),
            ..Seat::default()
        };
        assert_eq!(
            sessions_dir(&seat, Path::new("/Users/me/my.app")),
            Path::new("/c/projects/-Users-me-my-app")
        );
    }

    #[test]
    fn heavy_means_opus_as_the_main_model() {
        assert!(heavy(&json!({ "model": "opus" })));
        assert!(heavy(&json!({ "model": "claude-opus-4-1" })));
        assert!(!heavy(&json!({ "model": "opusplan" })));
        assert!(!heavy(&json!({})));
    }
}
