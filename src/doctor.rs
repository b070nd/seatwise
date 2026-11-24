//! `seat doctor [--fix]`: everything that would quietly weaken the isolation between seats, or stop a seat from working.

use crate::claude::HookState;
use crate::config::{Agent, Config, Enforcement, Link, Owner};
use crate::status::{Identity, SeatStatus};
use crate::{claude, folders, home, launch, paths, shim};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
}

#[derive(Debug)]
pub struct Finding {
    pub level: Level,
    pub message: String,
    /// What `--fix` would do about it, if anything.
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Install or remove the seat's hooks to match its enforcement level.
    Hooks(String),
    ReinstallShim,
    /// Remove folder entries whose folder doesn't exist.
    UnlinkMissing,
    /// Give the seat the desktop app's default data folder (`[seat.desktop] user_data_dir = "default"`).
    DesktopDefaultFolder(String),
    /// Bring the seat's seatwise skills (the `seatwise` plugin on Claude seats) up to date.
    Skills(String),
}

fn finding(level: Level, message: impl Into<String>) -> Finding {
    Finding {
        level,
        message: message.into(),
        fix: None,
    }
}

pub fn check(cfg: &Config, exe: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    if cfg.seats.is_empty() {
        out.push(finding(
            Level::Error,
            "no seats configured: `seat init` finds your Claude config folders, or `seat add <name>`",
        ));
    }
    for agent in [Agent::Claude, Agent::Codex] {
        if cfg.seats.iter().any(|s| s.agent == agent)
            && let Err(e) = launch::agent_binary(agent)
        {
            out.push(finding(Level::Error, format!("{e:#}")));
        }
    }

    let mut accounts: BTreeMap<(String, String), Vec<&str>> = BTreeMap::new();
    for seat in &cfg.seats {
        let n = &seat.name;
        if !cfg.is_enabled(seat) {
            out.push(finding(
                Level::Info,
                format!("seat '{n}' is disabled; it is not checked (`seat enable {n}`)"),
            ));
            continue;
        }
        let st = SeatStatus::of(cfg, seat, exe);
        if !seat.config_dir().is_dir() {
            out.push(finding(
                Level::Warn,
                format!(
                    "seat '{n}': config folder {} doesn't exist yet (`seat login {n}` creates it)",
                    seat.config_dir().display()
                ),
            ));
        }
        match &st.identity {
            Identity::SignedOut => out.push(finding(
                Level::Info,
                format!("seat '{n}' is not signed in: `seat login {n}`"),
            )),
            Identity::LoggedOut => out.push(finding(
                Level::Error,
                format!(
                    "seat '{n}' is signed out: its account is still recorded, but its login is gone (no \
                     .credentials.json or keychain item for its config folder): `seat login {n}`"
                ),
            )),
            Identity::Unpinned => out.push(finding(
                if st.enforcement == Enforcement::Off {
                    Level::Info
                } else {
                    Level::Warn
                },
                format!(
                    "seat '{n}' is signed in as {} but nothing is pinned, so a login to the wrong org goes unnoticed: `seat pin {n}`",
                    st.account_label()
                ),
            )),
            Identity::Wrong(m) => out.push(finding(Level::Error, m.clone())),
            Identity::Ok => {}
        }
        if !st.hooks_ok() {
            let message = match (st.enforcement, st.hooks) {
                (Enforcement::Off, _) => format!(
                    "seat '{n}': enforcement is off but seatwise hooks are still in its settings.json"
                ),
                (_, HookState::Missing) => format!(
                    "seat '{n}': enforcement is {} but its hooks are not installed, so nothing is checked",
                    st.enforcement.as_str()
                ),
                _ => match claude::hook_contract(seat) {
                    Some(c) if c != claude::HOOK_CONTRACT => format!(
                        "seat '{n}': its hooks were written for hook contract {c}, this seatwise speaks {}",
                        claude::HOOK_CONTRACT
                    ),
                    None => format!(
                        "seat '{n}': its hooks are from a seatwise before hook contracts (this one speaks {})",
                        claude::HOOK_CONTRACT
                    ),
                    Some(_) => {
                        format!(
                            "seat '{n}': its hooks point at another seatwise binary or seat name"
                        )
                    }
                },
            };
            let message = if seat.agent == Agent::Codex && st.enforcement != Enforcement::Off {
                message + " (Codex then asks you to approve them once in /hooks)"
            } else {
                message
            };
            out.push(Finding {
                level: if st.enforcement == Enforcement::Off {
                    Level::Info
                } else {
                    Level::Error
                },
                message,
                fix: Some(Fix::Hooks(n.clone())),
            });
        }
        let skills = crate::skills::state(seat, exe);
        let has_some = !skills.legacy.is_empty()
            || (skills.listed && seat.agent == Agent::Claude)
            || crate::skills::target(seat).exists();
        if has_some && !skills.current() {
            out.push(Finding {
                level: Level::Info,
                message: format!("seat '{n}': seatwise skills {}", skills.summary()),
                fix: Some(Fix::Skills(n.clone())),
            });
        }
        if let Some(a) = &st.account {
            let plan = crate::plan::Plan::of(a);
            if let (Some(pinned), Some(now)) = (&seat.plan, &plan)
                && *pinned != now.key()
            {
                out.push(finding(
                    Level::Warn,
                    format!(
                        "seat '{n}': its plan changed from {pinned} to {} (`seat pin {n}` accepts it, `seat profile {n}` suggests a fitting profile)",
                        now.key()
                    ),
                ));
            }
            if a.extra_usage {
                out.push(finding(
                    Level::Info,
                    format!("seat '{n}': extra usage is on, so use beyond the plan is billed"),
                ));
            }
            if let Some(t) = &a.trial_ends {
                out.push(finding(Level::Info, format!("seat '{n}': trial ends {t}")));
            }
            if seat.profile.is_none() && plan.is_some() && seat.agent == Agent::Claude {
                out.push(finding(
                    Level::Info,
                    format!("seat '{n}' has no usage profile: `seat profile {n}` suggests one for its plan"),
                ));
            }
        }
        if seat.agent == Agent::Codex && st.enforcement != Enforcement::Off && st.hooks_ok() {
            out.push(finding(
                Level::Info,
                format!("seat '{n}' runs Codex: approve the seatwise hooks once in Codex's /hooks, or they don't run"),
            ));
        }
        for path in home::foreign_mentions(cfg, seat) {
            out.push(finding(
                Level::Warn,
                format!("seat '{n}': its home repo mentions {path}, which leads its sessions into another seat's work"),
            ));
        }
        if let Some(h) = seat.home()
            && !h.is_dir()
        {
            out.push(finding(
                Level::Warn,
                format!(
                    "seat '{n}': home repo {} doesn't exist (`seat home init {n}`)",
                    h.display()
                ),
            ));
        }
        for item in &seat.share {
            if !cfg.shared_dir().join(item).exists() {
                out.push(finding(
                    Level::Warn,
                    format!(
                        "seat '{n}': shared item '{item}' is missing from {}",
                        cfg.shared_dir().display()
                    ),
                ));
            }
        }
        if let Some(a) = &st.account
            && let (Some(acc), Some(org)) = (&a.account_uuid, &a.org_uuid)
        {
            accounts
                .entry((acc.clone(), org.clone()))
                .or_default()
                .push(n);
        }
    }
    for seats in accounts.values().filter(|s| s.len() > 1) {
        out.push(finding(
            Level::Warn,
            format!(
                "seats {} are signed into the same account and org; one of them probably logged in to the wrong one",
                seats.join(", ")
            ),
        ));
    }

    out.extend(instruction_leaks(cfg));

    if !cfg.seats.is_empty()
        && cfg
            .seats
            .iter()
            .all(|s| cfg.enforcement_of(s) == Enforcement::Off)
    {
        out.push(finding(
            Level::Warn,
            "isolation is off: sessions can read every seat's folders. Set enforcement = \"warn\" (or \"strict\") and run `seat install-hooks`",
        ));
    } else {
        for s in cfg
            .seats
            .iter()
            .filter(|s| cfg.is_enabled(s) && cfg.enforcement_of(s) == Enforcement::Off)
        {
            out.push(finding(
                Level::Warn,
                format!(
                    "seat '{}' isn't isolated (enforcement off): its sessions can read every seat's folders",
                    s.name
                ),
            ));
        }
    }
    for f in &cfg.folders {
        if !paths::expand(&f.path).exists() {
            out.push(Finding {
                level: Level::Info,
                message: format!(
                    "folder {} doesn't exist (`seat unlink --missing` removes such entries)",
                    f.path
                ),
                fix: Some(Fix::UnlinkMissing),
            });
        }
    }
    for var in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        if std::env::var_os(var).is_some_and(|v| !v.is_empty()) {
            out.push(finding(
                Level::Info,
                format!("{var} is set in this shell: seatwise clears it for seats, but a plain `claude` would bill it instead of the seat"),
            ));
        }
    }
    out.extend(unused_default_and_logins(cfg));
    out.extend(settings_problems(cfg));
    for var in ["XDG_CONFIG_HOME", "XDG_DATA_HOME"] {
        if let Some(p) = std::env::var_os(var).map(PathBuf::from)
            && paths::session_scoped(&p)
        {
            out.push(finding(
                Level::Info,
                format!("{var} points into a temporary or session folder; seatwise ignores it and uses its usual folders"),
            ));
        }
    }
    let desktop = crate::desktop::findings(cfg, exe);
    for (n, m) in desktop.warnings.into_iter().enumerate() {
        out.push(Finding {
            level: Level::Warn,
            message: m,
            fix: match &desktop.default_folder_seat {
                Some(seat) if n == 0 => Some(Fix::DesktopDefaultFolder(seat.clone())),
                _ => None,
            },
        });
    }
    if exe.to_string_lossy().contains(' ') && cfg.seats.iter().any(|s| s.link() != Link::System) {
        out.push(finding(
            Level::Warn,
            format!(
                "{} has a space in its path; Claude Code may not run it as BROWSER. Install seat somewhere without spaces",
                exe.display()
            ),
        ));
    }
    if shim::installed() {
        let current = std::fs::read_to_string(shim::file()).ok() == Some(shim::script(exe));
        if !current {
            out.push(Finding {
                level: Level::Warn,
                message: format!(
                    "the claude shim in {} points at another seat binary",
                    shim::dir().display()
                ),
                fix: Some(Fix::ReinstallShim),
            });
        }
        let here = shim::first_on_path();
        match (here, !here && shim::first_in_login_shell() == Some(true)) {
            (true, _) => {}
            (false, true) => out.push(finding(
                Level::Info,
                "this process started before the claude shim was put first on PATH; new terminals have it. \
                 Restart this terminal or the desktop app (and tools started from it) to pick it up",
            )),
            _ => out.push(finding(
                Level::Warn,
                format!(
                    "the claude shim is installed but {} is not ahead of the real claude on PATH",
                    shim::dir().display()
                ),
            )),
        }
    }
    out.extend(nested_seats(cfg));
    out.extend(unguarded_codex(cfg));
    out.sort_by_key(|f| f.level);
    out
}

/// A seat's folder that contains folders of other seats. A tool that treats the outer folder as one project (an
/// agent orchestrator, an IDE workspace) runs everything in it on the outer seat, and the guard then blocks it in
/// the inner ones: correct, but confusing.
fn nested_seats(cfg: &Config) -> Vec<Finding> {
    let home = paths::home();
    let norm = |p: &str| folders::normalize(&paths::expand(p), &home);
    let mut out = Vec::new();
    for outer in &cfg.folders {
        let crate::config::Owner::Seat(a) = outer.owner() else {
            continue;
        };
        let ok = folders::key(&norm(&outer.path));
        let mut inner: Vec<&str> = cfg
            .folders
            .iter()
            .filter_map(|f| match f.owner() {
                crate::config::Owner::Seat(b) if b != a => {
                    let k = folders::key(&norm(&f.path));
                    (k.len() > ok.len() && k.starts_with(&ok)).then_some(b)
                }
                _ => None,
            })
            .collect();
        inner.sort();
        inner.dedup();
        if !inner.is_empty() {
            out.push(finding(
                Level::Info,
                format!(
                    "{} belongs to seat '{a}' but holds folders of seat {}: a tool that treats it as one project runs \
                     all of it on '{a}' and the guard blocks it in the others. Use one project per seat's folder",
                    paths::contract(&norm(&outer.path)),
                    inner.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
    out
}

/// `~/.codex` with no Codex seat on it: Codex sessions that tools start (an orchestrator, an IDE) use that login
/// with none of seatwise's guard.
fn unguarded_codex(cfg: &Config) -> Vec<Finding> {
    let dir = paths::home().join(".codex");
    let covered = cfg.seats.iter().any(|s| {
        s.agent == crate::config::Agent::Codex
            && folders::key_string(&s.config_dir()) == folders::key_string(&dir)
    });
    if covered || !dir.is_dir() {
        return Vec::new();
    }
    vec![finding(
        Level::Info,
        "~/.codex exists but no Codex seat uses it: Codex sessions started by tools (an orchestrator, an IDE) run \
         there without hooks or folder checks. To guard them: seat add codex --agent codex --config-dir ~/.codex",
    )]
}

/// Preset and profile settings the Settings Catalog finds wrong: unknown keys, bad values, keys Claude Code ignores
/// where the preset is applied. Experimental keys are fine here; `seat explain` mentions them.
fn settings_problems(cfg: &Config) -> Vec<Finding> {
    use crate::catalog::{Level as L, Target, check};
    let mut out = Vec::new();
    let mut push = |what: String, problems: Vec<crate::catalog::Problem>| {
        for p in problems {
            let level = match p.level {
                L::Error => Level::Error,
                L::Warn => Level::Warn,
                L::Info => continue,
            };
            out.push(finding(level, format!("{what}: {}", p.message)));
        }
    };
    for p in crate::preset::all(cfg)
        .iter()
        .filter(|p| p.enabled && !p.builtin)
    {
        let s = crate::preset::settings(cfg, p);
        let mut problems = check(&s, Target::Session);
        if cfg.groups.iter().any(|g| {
            g.enabled
                && g.preset == p.name
                && g.apply.unwrap_or(cfg.preset_apply) == crate::config::Apply::Project
        }) {
            for q in check(&s, Target::Local) {
                if !problems.contains(&q) {
                    problems.push(q);
                }
            }
        }
        push(format!("preset '{}'", p.name), problems);
    }
    for name in cfg.profiles.keys() {
        if let Some(s) = crate::plan::settings_of(cfg, name) {
            push(format!("profile '{name}'"), check(&s, Target::Session));
        }
    }
    out
}

/// Claude Code's default config folder coming back when no seat uses it (a plain `claude`, or the desktop app
/// started from the Dock, recreates it: signed out, without hooks), and logins left in the keychain for config
/// folders no seat uses (a retired `~/.claude`, say). Only the names of keychain items are looked at.
fn unused_default_and_logins(cfg: &Config) -> Vec<Finding> {
    let mut out = Vec::new();
    let claude_seats: Vec<&crate::config::Seat> = cfg
        .seats
        .iter()
        .filter(|s| s.agent == crate::config::Agent::Claude)
        .collect();
    if claude_seats.is_empty() {
        return out;
    }
    let home = paths::home();
    let default_dir = home.join(".claude");
    let default_used = claude_seats.iter().any(|s| s.is_default_location());
    if !default_used && default_dir.is_dir() {
        // Only state or instructions prove a session ran there; the desktop app and some tools create empty
        // folders (`bridge-spawn`, `ssh`) or drop a skill in whatever the seat.
        let used = [
            "settings.json",
            "CLAUDE.md",
            "projects",
            "history.jsonl",
            ".credentials.json",
        ]
        .iter()
        .any(|n| default_dir.join(n).exists())
            || home.join(".claude.json").exists();
        if used {
            out.push(finding(
                Level::Warn,
                format!(
                    "{} holds settings or sessions but no seat uses it: something started Claude Code without a seat \
                     (a plain `claude`, or the desktop app from the Dock or Finder). Sessions there have no seat, \
                     hooks or pinned org",
                    paths::contract(&default_dir)
                ),
            ));
        } else {
            let mut names: Vec<String> = std::fs::read_dir(&default_dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            out.push(finding(
                Level::Info,
                format!(
                    "{} holds no settings or sessions, only: {} (created by the desktop app or a tool){}",
                    paths::contract(&default_dir),
                    if names.is_empty() { "nothing".into() } else { names.join(", ") },
                    if names.iter().any(|n| n == "skills") {
                        ". Skills there reach no seat; copy them with `seat share --adopt`"
                    } else {
                        ""
                    }
                ),
            ));
        }
    }
    let mut candidates = vec![(default_dir.clone(), true)];
    for e in std::fs::read_dir(&home).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(".claude-") && e.path().is_dir() {
            candidates.push((e.path(), false));
        }
    }
    for (dir, default_location) in candidates {
        let used = claude_seats.iter().any(|s| {
            crate::folders::key_string(&s.config_dir()) == crate::folders::key_string(&dir)
        });
        if used || (default_location && default_used) {
            continue;
        }
        let service = crate::usage::keychain_service_for(&dir, default_location);
        if crate::usage::keychain_has(&service) {
            // Never a removal command for a login that may be in use: the folder still exists (another config,
            // a tool, plain `claude`), or it is the default login every seatless `claude` uses.
            out.push(finding(Level::Info, unused_login(&dir, default_location)));
        }
    }
    out
}

/// What to say about a keychain login no seat in this config uses. It may still be live (used by plain `claude`,
/// another config or a tool), so doctor only says how to use it; it never suggests removing it.
fn unused_login(dir: &Path, default_location: bool) -> String {
    let shown = paths::contract(dir);
    if default_location {
        format!(
            "the keychain holds the default Claude Code login (for {shown}), which no seat in this config uses; \
             plain `claude` outside seatwise still uses it"
        )
    } else if dir.exists() {
        format!(
            "the keychain holds a Claude Code login for {shown}, which no seat in this config uses: \
             `seat add <name> --config-dir {shown}` to make it a seat"
        )
    } else {
        format!(
            "the keychain holds a Claude Code login for {shown}, which no longer exists and no seat uses. \
             If you retired that folder, you can remove the login with Keychain Access"
        )
    }
}

/// Claude Code reads `.claude/CLAUDE.md` from every parent of the working folder. One sitting above another
/// seat's folders (typically a seat living in `~/.claude`) is loaded into that seat's sessions too.
fn instruction_leaks(cfg: &Config) -> Vec<Finding> {
    let home = paths::home();
    let mut by_file: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for f in &cfg.folders {
        let folder = folders::normalize(&paths::expand(&f.path), &home);
        let affected: Vec<&str> = match f.owner() {
            Owner::Seat(s) => vec![s],
            Owner::Any => cfg.seats.iter().map(|s| s.name.as_str()).collect(),
            Owner::Nobody => continue,
        };
        // `.claude/CLAUDE.md` is Claude Code's; Codex seats don't load it.
        let affected: Vec<&str> = affected
            .into_iter()
            .filter(|s| cfg.seat(s).is_some_and(|x| x.agent == Agent::Claude))
            .collect();
        for ancestor in folder.ancestors() {
            let md = ancestor.join(".claude").join("CLAUDE.md");
            if !md.is_file() {
                continue;
            }
            let own = folders::key_string(&ancestor.join(".claude"));
            for s in &affected {
                let seat = cfg.seat(s).expect("validated config");
                if folders::key_string(&seat.config_dir()) != own {
                    by_file.entry(md.clone()).or_default().push(s.to_string());
                }
            }
        }
    }
    by_file
        .into_iter()
        .map(|(md, mut seats)| {
            seats.sort();
            seats.dedup();
            finding(
                Level::Warn,
                format!(
                    "{} sits above folders of seat(s) {} and is loaded into their sessions. Move it, or give the seat that owns it its own config_dir",
                    md.display(),
                    seats.join(", ")
                ),
            )
        })
        .collect()
}

/// Apply the fixes doctor knows. Returns what was done.
pub fn fix(cfg: &Config, exe: &Path, findings: &[Finding]) -> anyhow::Result<Vec<String>> {
    let mut done = Vec::new();
    for f in findings {
        match &f.fix {
            Some(Fix::Hooks(name)) => {
                let seat = cfg.seat(name).expect("validated config");
                let on = claude::install_hooks(cfg, seat, exe)?;
                done.push(format!(
                    "seat '{name}': hooks {}",
                    if on { "installed" } else { "removed" }
                ));
            }
            Some(Fix::ReinstallShim) => {
                shim::install(exe)?;
                done.push("claude shim reinstalled".into());
            }
            Some(Fix::UnlinkMissing) => {
                if done.iter().any(|d| d.ends_with("(folder doesn't exist)")) {
                    continue; // one pass removes them all
                }
                let mut file = crate::edit::ConfigFile::open()?;
                let removed = file.unlink_missing();
                if !removed.is_empty() {
                    file.save()?;
                }
                done.extend(
                    removed
                        .into_iter()
                        .map(|p| format!("{p}: entry removed (folder doesn't exist)")),
                );
            }
            Some(Fix::DesktopDefaultFolder(name)) => {
                let mut file = crate::edit::ConfigFile::open()?;
                file.set_seat_desktop(name, "user_data_dir", "default")?;
                file.save()?;
                done.push(format!(
                    "seat '{name}': desktop uses the app's default data folder (user_data_dir = \"default\")"
                ));
            }
            Some(Fix::Skills(name)) => {
                let seat = cfg.seat(name).expect("validated config");
                let installed = crate::skills::install(cfg, seat, exe)?;
                done.push(format!(
                    "seat '{name}': skills installed{}",
                    crate::hybrid::install_note(&installed)
                ));
            }
            None => {}
        }
    }
    Ok(done)
}

pub fn report(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "all good\n".into();
    }
    let mut out = String::new();
    for f in findings {
        let tag = match f.level {
            Level::Error => "error",
            Level::Warn => "warn ",
            Level::Info => "info ",
        };
        let fixable = if f.fix.is_some() { " [--fix]" } else { "" };
        out += &format!("{tag}  {}{fixable}\n", f.message);
    }
    out
}
