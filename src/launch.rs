//! `seat run <seat|auto> [agent args...]`: start Claude Code (or Codex) on the right seat, or refuse.

use crate::config::{Agent, Config, Enforcement, Link, Owner, Seat};
use crate::folders::{normalize, owner_of};
use crate::{claude, guard, paths, share, sync};
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Extras for one start.
#[derive(Debug, Default, Clone)]
pub struct RunOptions {
    /// Usage profiles applied to this session only (`--settings`), not written to settings.json.
    pub with: Vec<String>,
    /// Start in the seat's home repo.
    pub home: bool,
}

pub fn run(cfg: &Config, seat_arg: &str, opts: &RunOptions, args: &[String]) -> Result<i32> {
    if opts.home {
        let seat = cfg
            .seat(seat_arg)
            .context("--home needs a seat name, not auto")?;
        let home = seat.home().with_context(|| {
            format!(
                "seat '{}' has no home repo (`seat home init {}`)",
                seat.name, seat.name
            )
        })?;
        std::env::set_current_dir(&home).with_context(|| format!("entering {}", home.display()))?;
    }
    let cwd = normalize(&std::env::current_dir()?, &paths::home());
    let owner = owner_of(cfg, &cwd);
    // `claude --version` and `--help` start no session: tools run them from their own folders (a model list, an
    // update check). In a folder no seat owns they get a seat's setup without the folder check.
    let probe_only = seat_arg == "auto"
        && matches!(
            args.first().map(String::as_str),
            Some("--version" | "-v" | "--help" | "-h")
        )
        && !matches!(owner, Some(Owner::Seat(_)));
    let seat = if probe_only {
        probe_seat(cfg).context("no enabled Claude Code seat to run it on")?
    } else if seat_arg == "auto" {
        resolve_auto(cfg, owner, &cwd)?
    } else {
        cfg.seat_named(seat_arg)?
    };
    if !cfg.is_enabled(seat) {
        let why = if seat.enabled {
            format!(
                "the {} agent is turned off (`seat enable agent:{}`)",
                seat.agent.as_str(),
                seat.agent.as_str()
            )
        } else {
            format!("it is disabled (`seat enable {}`)", seat.name)
        };
        bail!("seat '{}' can't start: {why}", seat.name);
    }

    if !guard::forced() && !probe_only {
        let level = cfg.enforcement_of(seat);
        if level != Enforcement::Off {
            apply(level, guard::check_start(cfg, seat, &cwd))?;
        }
        // Only the state file counts here: an exported CLAUDE_CODE_ORGANIZATION_UUID belongs to whatever started us.
        // A wrong org is always worth a warning (same email in two orgs makes /login land on the wrong one easily).
        let account = claude::read_account(seat).unwrap_or_default();
        let org_level = if level == Enforcement::Strict {
            level
        } else {
            Enforcement::Warn
        };
        apply(
            org_level,
            guard::check_identity(seat, account.org_uuid.as_deref(), account.email.as_deref()),
        )?;
    }
    for note in sync::reconcile(seat, &cfg.track)? {
        eprintln!("seat: {note}");
    }
    for note in share::apply(cfg, seat)? {
        eprintln!("seat: {note}");
    }
    for note in crate::home::apply_skills(seat)? {
        eprintln!("seat: {note}");
    }
    if let Some(t) = crate::compact::enforce(cfg, seat)? {
        eprintln!(
            "seat: compaction at {} set in {}",
            crate::compact::label(t),
            seat.hooks_file().display()
        );
    }

    crate::sentinel::on_start(cfg, seat, &cwd);

    let mut cmd = seat_command(seat)?;
    let resolved = crate::preset::resolve(cfg, seat, &cwd);
    let mut session = json!({});
    if let Some(name) = &resolved.preset {
        match resolved.apply {
            crate::config::Apply::Session => session = resolved.settings.clone(),
            crate::config::Apply::Project => {
                let root = crate::preset::project_root(&cwd);
                if crate::preset::write_project(&root, &resolved.settings)? {
                    eprintln!(
                        "seat: preset '{name}' written to {}",
                        root.join(".claude").join("settings.local.json").display()
                    );
                }
            }
        }
    }
    if let Some(sources) = &resolved.sources {
        cmd.arg("--setting-sources").arg(sources.join(","));
    }
    if owner == Some(Owner::Any) && seat.agent == Agent::Claude {
        // From the shared start folder, the seat can work in its own folders without being asked.
        let dirs: Vec<String> = cfg
            .folders_of(&seat.name)
            .into_iter()
            .filter(|p| p.is_dir())
            .map(|p| p.display().to_string())
            .collect();
        if !dirs.is_empty() {
            crate::preset::merge_deep(
                &mut session,
                &json!({ "permissions": { "additionalDirectories": dirs } }),
            );
        }
    }
    // The enforced compaction window beats project and local settings; a `--with` tier below wins for this session.
    if let Some(t) = crate::compact::of(cfg, seat) {
        crate::preset::merge_deep(&mut session, &json!({ crate::compact::SETTING: t }));
    }
    for name in &opts.with {
        if seat.agent != Agent::Claude {
            bail!("session profiles are for Claude Code seats; Codex has its own `--profile`");
        }
        let profile = crate::plan::settings_of(cfg, name)
            .with_context(|| format!("unknown profile '{name}'"))?;
        crate::plan::merge(&mut session, &profile);
    }
    // Variables Claude Code reads only from the launch environment can't travel in `--settings`: export them.
    if let Some(env) = session["env"].as_object() {
        for (k, v) in env {
            if crate::catalog::launch_only(k) {
                cmd.env(k, v.as_str().map_or_else(|| v.to_string(), str::to_string));
            }
        }
    }
    if session.as_object().is_some_and(|o| !o.is_empty()) {
        cmd.arg("--settings").arg(session.to_string());
    }
    if seat.agent == Agent::Claude {
        // Same folder and config → same fingerprint; `seat ps` shows it.
        let print = crate::preset::fingerprint(&session, resolved.sources.as_deref());
        let name = resolved.preset.as_deref().unwrap_or("-");
        cmd.env("SEATWISE_PRESET", format!("{name}@{print}"));
    }
    cmd.args(args);
    exec(cmd)
}

/// The agent with a seat's environment: its config folder, its link handling, and no stray credentials.
pub fn seat_command(seat: &Seat) -> Result<Command> {
    let mut cmd = Command::new(agent_binary(seat.agent)?);
    let var = seat.agent.home_var();
    if seat.is_default_location() {
        cmd.env_remove(var);
    } else {
        // Codex wants the folder to exist.
        std::fs::create_dir_all(seat.config_dir())?;
        cmd.env(var, seat.config_dir());
    }
    // Claude Code opens links with `$BROWSER <url>`; route them through `seat <url>` unless the seat uses the
    // system browser, in which case the user's own BROWSER (if any) is left alone.
    if seat.link() != Link::System {
        cmd.env("BROWSER", std::env::current_exe()?);
    }
    cmd.env("SEATWISE_SEAT", &seat.name);
    // Started from inside another agent session (a desktop Code session, a CLI session running `seat term`): that
    // session's own wiring must not reach the new one. The desktop app, for one, points ANTHROPIC_BASE_URL at itself.
    // A base URL set in your own shell (outside any session) is left alone.
    if std::env::var_os("CLAUDECODE").is_some()
        || std::env::var_os("CLAUDE_CODE_ENTRYPOINT").is_some()
    {
        for inherited in [
            "ANTHROPIC_BASE_URL",
            "CLAUDECODE",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_SSE_PORT",
        ] {
            cmd.env_remove(inherited);
        }
    }
    for leaked in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_ORGANIZATION_UUID",
        "OPENAI_API_KEY",
        // The other agent's folder variable, from whatever started us.
        match seat.agent {
            Agent::Claude => "CODEX_HOME",
            Agent::Codex => "CLAUDE_CONFIG_DIR",
        },
    ] {
        cmd.env_remove(leaked);
    }
    Ok(cmd)
}

/// `seat login <seat>`: the agent's own login in the seat's environment (no folder checks: it may be the first run).
pub fn login(seat: &Seat) -> Result<i32> {
    std::fs::create_dir_all(seat.config_dir())?;
    let args: &[&str] = match seat.agent {
        Agent::Claude => &["auth", "login"],
        Agent::Codex => &["login"],
    };
    let status = seat_command(seat)?
        .args(args)
        .status()
        .with_context(|| format!("starting {}", seat.agent.binary()))?;
    Ok(status.code().unwrap_or(1))
}

fn apply(level: Enforcement, check: Result<(), String>) -> Result<()> {
    match (check, level) {
        (Ok(()), _) | (Err(_), Enforcement::Off) => Ok(()),
        (Err(m), Enforcement::Warn) => {
            eprintln!("seat: warning: {m}");
            Ok(())
        }
        (Err(m), Enforcement::Strict) => bail!("{m}"),
    }
}

/// The seat a probe (`claude --version`) runs on: `default_seat`, else the first enabled Claude Code seat.
fn probe_seat(cfg: &Config) -> Option<&Seat> {
    cfg.default_seat
        .as_deref()
        .and_then(|d| cfg.seat(d))
        .filter(|s| cfg.is_enabled(s) && s.agent == Agent::Claude)
        .or_else(|| {
            cfg.seats
                .iter()
                .find(|s| cfg.is_enabled(s) && s.agent == Agent::Claude)
        })
}

/// The seat for `seat run auto`: the folder's owner, else `default_seat`, else a choice on the terminal.
fn resolve_auto<'a>(cfg: &'a Config, owner: Option<Owner<'_>>, cwd: &Path) -> Result<&'a Seat> {
    let why = match owner {
        Some(Owner::Seat(s)) => {
            let seat = cfg.seat(s).expect("validated config");
            if cfg.is_enabled(seat) {
                return Ok(seat);
            }
            format!("{} belongs to seat '{s}', which is disabled", cwd.display())
        }
        Some(Owner::Any) => format!("{} is a shared start folder", cwd.display()),
        Some(Owner::Nobody) | None => {
            if cfg.enforcement == Enforcement::Strict {
                // Reuse the wording of the start check; the folder belongs to no seat anyway.
                let msg = match cfg.seats.first() {
                    Some(seat) => guard::check_start(cfg, seat, cwd).err().unwrap_or_default(),
                    None => "no seats configured".into(),
                };
                bail!("{msg}");
            }
            if let Some(d) = &cfg.default_seat
                && cfg.is_enabled(cfg.seat(d).expect("validated config"))
            {
                return Ok(cfg.seat(d).expect("validated config"));
            }
            format!("{} belongs to no seat", cwd.display())
        }
    };
    if cfg.seats.is_empty() {
        bail!("no seats configured in {}", paths::config_file().display());
    }
    if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        return pick(cfg, &why);
    }
    bail!(
        "{why}; pick a seat ({}), or set default_seat in {}",
        cfg.seat_names(),
        paths::config_file().display()
    )
}

fn pick<'a>(cfg: &'a Config, why: &str) -> Result<&'a Seat> {
    eprintln!("seat: {why}. Which seat?");
    let seats: Vec<&Seat> = cfg.seats.iter().filter(|s| cfg.is_enabled(s)).collect();
    if seats.is_empty() {
        bail!("every seat is disabled");
    }
    for (i, seat) in seats.iter().enumerate() {
        eprintln!("  {}) {}", i + 1, seat.name);
    }
    loop {
        eprint!("> ");
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line)? == 0 {
            bail!("no seat chosen");
        }
        let answer = line.trim();
        let by_number = answer
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| seats.get(i).copied());
        if let Some(seat) = by_number.or_else(|| cfg.seat(answer).filter(|s| cfg.is_enabled(s))) {
            return Ok(seat);
        }
    }
}

/// The real `claude`, skipping ourselves and seatwise's own `claude` shim.
pub fn claude_binary() -> Result<PathBuf> {
    agent_binary(Agent::Claude)
}

/// The agent's real binary (`SEATWISE_CLAUDE` / `SEATWISE_CODEX` override), skipping ourselves and the shim folder.
pub fn agent_binary(agent: Agent) -> Result<PathBuf> {
    let override_var = match agent {
        Agent::Claude => "SEATWISE_CLAUDE",
        Agent::Codex => "SEATWISE_CODEX",
    };
    if let Some(p) = std::env::var_os(override_var) {
        return Ok(p.into());
    }
    let name = agent.binary();
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| dunce::canonicalize(p).ok());
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    let shim_dir = crate::folders::key_string(&crate::shim::dir());
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if crate::folders::key_string(&dir) == shim_dir {
            continue;
        }
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() && dunce::canonicalize(&p).ok() != me {
                return Ok(p);
            }
        }
    }
    bail!("{name} was not found in PATH (install it, or set {override_var})")
}

#[cfg(unix)]
fn exec(mut cmd: Command) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    Err(cmd.exec()).context("starting claude")
}

#[cfg(windows)]
fn exec(mut cmd: Command) -> Result<i32> {
    // Claude gets Ctrl-C from the shared console; we must not die first and orphan it.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 1);
    }
    let status = cmd.status().context("starting claude")?;
    Ok(status.code().unwrap_or(1))
}
