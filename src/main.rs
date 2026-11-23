use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use seatwise::config::{Config, Owner, Seat};
use seatwise::edit::ConfigFile;
use seatwise::guard::Verdict;
use seatwise::plan::{self, Plan};
use seatwise::{
    claude, doctor, folders, guard, init, launch, link, paths, ps, share, shell, shim, status,
    sync, usage,
};
use std::io::IsTerminal;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "seat",
    version,
    about = "Keep Claude Code accounts (seats) out of each other's work: own config, org and folders per seat"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a seat's agent ("auto": the seat that owns this folder). Further arguments go to claude / codex.
    Run {
        /// A usage profile for this session only, e.g. `--with experimental` (repeatable)
        #[arg(long = "with")]
        with: Vec<String>,
        /// Start in the seat's home repo
        #[arg(long)]
        home: bool,
        seat: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Propose a config from the Claude Code folders on this machine (and write it, if there is none yet)
    Init {
        /// Only print the proposal
        #[arg(long)]
        print: bool,
        /// Replace an existing config file
        #[arg(long)]
        force: bool,
    },
    /// Add a seat
    Add {
        name: String,
        /// Its Claude Code config folder (default: ~/.claude-<name>); an existing one is adopted as is
        #[arg(long)]
        config_dir: Option<String>,
        /// Other names for it (each also gets a shell command)
        #[arg(long = "alias")]
        aliases: Vec<String>,
        /// Start with a copy of this seat's setup: settings, CLAUDE.md, skills, agents, commands, plugins (never its login)
        #[arg(long)]
        from: Option<String>,
        /// "claude" (default) or "codex" (OpenAI Codex CLI, for ChatGPT plans)
        #[arg(long, default_value = "claude")]
        agent: String,
    },
    /// Open a terminal window with the CLI for this project (or --dir) on its seat; for working next to the desktop app
    Term {
        /// The seat (default: the seat of the session this runs in, else the folder's owner)
        #[arg(long)]
        seat: Option<String>,
        /// Usage profile for that session only (default: terminal.profile in the config)
        #[arg(long = "with")]
        with: Vec<String>,
        /// Start in the seat's home repo
        #[arg(long)]
        home: bool,
        /// The project folder (default: $CLAUDE_PROJECT_DIR, else this folder)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Only print the command
        #[arg(long)]
        print: bool,
        /// Passed on to the CLI, after `--`
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Start the Claude desktop app on a seat (macOS): its own data folder and the seat's environment. Without a
    /// seat, list the running desktop instances
    Desktop {
        /// The seat (or one of its aliases)
        seat: Option<String>,
    },
    /// Set up the command line for this project next to the desktop app (seat, folder, skills, preset), then
    /// open a terminal on the same seat and project
    Hybrid {
        /// The seat (default: the seat of the session this runs in, else the folder's owner)
        #[arg(long)]
        seat: Option<String>,
        /// The project folder (default: $CLAUDE_PROJECT_DIR, else this folder)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Session-only settings for the terminal, e.g. `--with experimental` (default: terminal.profile)
        #[arg(long = "with")]
        with: Vec<String>,
        /// Give the project this use-case preset (explore, build, review, quick, or your own)
        #[arg(long)]
        preset: Option<String>,
        /// Give an unassigned project folder to the seat
        #[arg(long)]
        link: bool,
        /// Only report; change nothing, open nothing
        #[arg(long)]
        check: bool,
        /// Set up, but don't open a terminal
        #[arg(long)]
        no_open: bool,
        /// Print the terminal's command instead of opening it
        #[arg(long)]
        print: bool,
        /// Print the plan as JSON (steps, and the open choices with their flags); changes nothing
        #[arg(long)]
        json: bool,
    },
    /// The seat's home repo: one repo with its skills, AGENTS.md and pointers to its other repos
    #[command(subcommand)]
    Home(HomeCmd),
    /// The seatwise skills (open a terminal for the project, seat status, work from the home repo)
    #[command(subcommand)]
    Skills(SkillsCmd),
    /// Sign a seat in (`claude auth login` in the seat's environment), then pin the account if nothing is pinned
    Login { seat: String },
    /// Record the org and email a seat is signed into, so a login to the wrong one is caught
    Pin { seat: String },
    /// A seat's plan and usage profile: without a name, show the profile that fits the plan; with one, apply it
    Profile {
        seat: String,
        name: Option<String>,
        /// Also look at this week's usage (asks the usage endpoint with the seat's login)
        #[arg(long)]
        usage: bool,
    },
    /// How much of each seat's 5-hour and weekly limits is used (asks Anthropic with each seat's own login)
    Usage { seats: Vec<String> },
    /// Give a folder (default: this one) to a seat, or make it shared ("any") or closed ("none")
    Link {
        #[arg(required_unless_present = "from")]
        owner: Option<String>,
        path: Option<PathBuf>,
        /// Read many at once from a file: one `<owner> <path>` per line (`#` comments and blank lines are skipped;
        /// owner is a seat, "any" or "none"). Nothing is written unless every line is valid
        #[arg(long, conflicts_with_all = ["owner", "path"])]
        from: Option<PathBuf>,
    },
    /// Remove a folder's entry (default: this one)
    Unlink {
        path: Option<PathBuf>,
        /// Remove every entry whose folder no longer exists (names no paths, so it also works inside a guarded
        /// session). A folder on a drive that isn't mounted counts as missing.
        #[arg(long, conflicts_with = "path")]
        missing: bool,
    },
    /// Show which seat owns a folder (default: the current one)
    Owner { path: Option<PathBuf> },
    /// Every seat: account, identity check, enforcement, hooks, links, folders
    #[command(alias = "status")]
    List,
    /// One seat in full: account, plan, identity, isolation, links, desktop app, and its projects
    Show {
        seat: String,
        #[arg(long)]
        json: bool,
    },
    /// Every project folder: its seat, preset, sessions and when it was last used
    Projects {
        #[arg(long)]
        json: bool,
    },
    /// Claude Code sessions running now, by seat
    Ps,
    /// Check for anything that weakens the isolation between seats or stops one from working
    Doctor {
        /// Apply the fixes doctor knows (hooks, shim)
        #[arg(long)]
        fix: bool,
    },
    /// Install or refresh the guard hooks in each seat's settings.json, or remove them where enforcement is off
    /// (all seats if none are named)
    InstallHooks { seats: Vec<String> },
    /// Copy each seat's `share` items from the shared folder now (also done on every start)
    Share { seats: Vec<String> },
    /// Print shell functions `claude-<seat>` (and `claude`, if enabled); e.g. `eval "$(seat shell-init zsh)"` in ~/.zshrc
    ShellInit { shell: shell::Shell },
    /// A `claude` command on PATH that picks the seat by folder, for IDEs and tools that start claude themselves
    #[command(subcommand)]
    Shim(ShimCmd),
    /// Print shell completions
    Completions { shell: clap_complete::Shell },
    /// Handle a link the way the current seat's `link` setting says (BROWSER points here)
    Url {
        url: String,
        /// Copy it, whatever the seat's setting
        #[arg(long)]
        copy: bool,
    },
    /// Hooks run by Claude Code (installed by install-hooks)
    #[command(subcommand)]
    Hook(HookCmd),
    /// Settings snapshots
    #[command(subcommand)]
    Sync(SyncCmd),
    /// Print the config file location
    ConfigPath,
    /// Turn on a seat, agent, preset, folder group or the sentinel: `work`, `agent:codex`, `preset:review`,
    /// `group:legacy`, `sentinel`
    Enable { target: String },
    /// Turn one off (its config stays)
    Disable { target: String },
    /// What a session in a folder (default: this one) would run with: its group, preset, settings layers, and
    /// which layer set each value
    Explain {
        path: Option<PathBuf>,
        #[arg(long)]
        seat: Option<String>,
        #[arg(long = "with")]
        with: Vec<String>,
    },
    /// Use-case presets (usage profiles combined with settings) for folder groups
    #[command(subcommand)]
    Preset(PresetCmd),
    /// Every Claude Code setting and environment variable: find them, read what they do, check presets
    #[command(subcommand)]
    Settings(SettingsCmd),
    /// Folder groups: folders that share a preset
    #[command(subcommand)]
    Group(GroupCmd),
    /// Look for settings that cost more than they need to (never changes anything)
    Sentinel(SentinelArgs),
    /// Full-screen settings: seats, agents, presets, folder groups and the sentinel
    Tui,
}

#[derive(Subcommand)]
enum SettingsCmd {
    /// Settings and environment variables whose name or meaning contains every word
    Search {
        query: Vec<String>,
        /// Only experimental ones
        #[arg(long)]
        experimental: bool,
        /// Only environment variables
        #[arg(long)]
        env: bool,
        /// Include deprecated and removed ones
        #[arg(long)]
        all: bool,
    },
    /// What a setting or environment variable does, where Claude Code honors it, and how to set it in a preset
    Show { key: String },
    /// Check a preset's or profile's settings (or a settings JSON file) against the catalog
    Check {
        /// A preset or profile name
        name: Option<String>,
        /// A settings JSON file instead
        #[arg(long, conflicts_with = "name")]
        file: Option<PathBuf>,
        /// Where the settings are applied: session (default for presets), local (apply = "project"), user
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Subcommand)]
enum PresetCmd {
    /// Every preset: built-in and your own
    List,
    /// A preset's resolved settings
    Show { name: String },
    /// Build a new preset step by step (a wizard in a terminal); only valid choices are taken
    New {
        name: String,
        /// Start from a preset or a profile
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Ask step by step even when input isn't a terminal
        #[arg(long)]
        interactive: bool,
    },
    /// Change a preset step by step (a wizard)
    Edit { name: String },
    /// Set one setting or environment variable in a preset; refused when Claude Code wouldn't accept it there
    Set {
        name: String,
        key: String,
        value: String,
    },
    /// Remove a setting or environment variable from a preset
    Unset { name: String, key: String },
    /// The usage profiles a preset builds on, in order (none: clear them)
    Profiles { name: String, profiles: Vec<String> },
    /// The settings a preset can take now, with their values (what the wizard offers)
    Options {
        name: String,
        query: Vec<String>,
        #[arg(long)]
        experimental: bool,
    },
}

#[derive(Subcommand)]
enum GroupCmd {
    /// Every folder group
    List,
    /// Add a group for a folder (default: this one); asks for what isn't given, on a terminal
    Add {
        path: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        /// A preset name, or "none" (this folder opts out of a parent group's preset)
        #[arg(long)]
        preset: Option<String>,
        /// "session" (passed at start) or "project" (written to .claude/settings.local.json)
        #[arg(long)]
        apply: Option<String>,
        /// Claude Code setting sources to load here, e.g. "user,local" (ignores the project's settings.json)
        #[arg(long)]
        sources: Option<String>,
    },
    /// Remove a group
    Remove { name: String },
}

#[derive(clap::Args)]
struct SentinelArgs {
    #[command(subcommand)]
    cmd: Option<SentinelCmd>,
    /// Only these seats (default: every enabled seat and its projects)
    seats: Vec<String>,
    /// Only this folder's project and seat
    #[arg(long)]
    here: bool,
    /// Also show a desktop notification
    #[arg(long)]
    notify: bool,
    /// Print nothing (for scheduled runs)
    #[arg(long)]
    quiet: bool,
}

#[derive(Subcommand)]
enum SentinelCmd {
    /// Run it on a schedule (sentinel.interval): launchd on macOS, a systemd user timer on Linux, Task Scheduler on Windows
    Install {
        /// Only show what would be written and run
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the schedule
    Uninstall {
        #[arg(long)]
        dry_run: bool,
    },
    /// The rules it checks
    Rules,
}

#[derive(Subcommand)]
enum HookCmd {
    /// UserPromptSubmit: prompts in another seat's folder or on the wrong org
    Prompt { seat: String },
    /// PreToolUse: tool calls that touch another seat's folders
    Tool { seat: String },
}

#[derive(Subcommand)]
enum HomeCmd {
    /// Create (or adopt) the home repo, give it to the seat, and write its pointers
    Init { seat: String, path: Option<PathBuf> },
    /// Rewrite the "Other repos" pointers from the seat's folders (all seats with a home repo if none named)
    Sync { seats: Vec<String> },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// Install the skills for these seats (all if none named)
    Install { seats: Vec<String> },
    /// Print one skill as it would be installed
    Show { name: String },
}

#[derive(Subcommand)]
enum ShimCmd {
    /// Write the shim; then put its folder first on PATH
    Install,
    Uninstall,
}

#[derive(Subcommand)]
enum SyncCmd {
    /// Swap the tracked files back to their previous version (run again to undo)
    Restore { seat: String },
}

/// A draft of a preset to change: yours, or a built-in one (changing it writes your own copy).
fn draft_of(cfg: &Config, name: &str) -> Result<seatwise::draft::Draft> {
    seatwise::draft::Draft::from_preset(cfg, name, name)
}

/// Run the wizard on a draft in a terminal (or when asked), else write the draft as it is.
fn preset_session(cfg: &Config, d: seatwise::draft::Draft, interactive: bool) -> Result<i32> {
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let d = if terminal || interactive {
        let stdin = std::io::stdin();
        match seatwise::wizard::run(cfg, d, &mut stdin.lock(), &mut std::io::stdout())? {
            Some(d) => d,
            None => {
                println!("nothing written");
                return Ok(1);
            }
        }
    } else {
        d
    };
    let mut file = ConfigFile::open()?;
    d.save(cfg, &mut file)?;
    file.save()?;
    println!(
        "preset '{}' written; `seat group add <folder> --preset {}` gives it to folders",
        d.name, d.name
    );
    Ok(0)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // Test double: the integration tests copy this binary as `fake-claude` to see what `seat run` hands to Claude.
    if std::env::current_exe().is_ok_and(|p| {
        p.file_stem()
            .is_some_and(|s| s == "fake-claude" || s == "fake-codex")
    }) {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        println!("CFG={}", var("CLAUDE_CONFIG_DIR"));
        println!("CODEX={}", var("CODEX_HOME"));
        println!("SEAT={}", var("SEATWISE_SEAT"));
        println!("BROWSER={}", var("BROWSER"));
        println!("PRESET={}", var("SEATWISE_PRESET"));
        println!("BASE={}", var("ANTHROPIC_BASE_URL"));
        println!("DISKWARN={}", var("CLAUDE_CODE_WARN_DISK_FREE_GIB"));
        let key = std::env::var_os("ANTHROPIC_API_KEY").is_some();
        println!("KEY={}", if key { "set" } else { "none" });
        println!("ARGS={}", argv[1..].join(" "));
        // `claude plugin install <id>` records the plugin the way Claude Code does.
        if argv.get(1..3).is_some_and(|a| a == ["plugin", "install"]) {
            let dir = PathBuf::from(var("CLAUDE_CONFIG_DIR")).join("plugins");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(
                dir.join("installed_plugins.json"),
                format!(
                    "{{\"plugins\": {{\"{}\": [{{\"scope\": \"user\"}}]}}}}",
                    argv[3]
                ),
            );
        }
        return ExitCode::SUCCESS;
    }
    // Claude Code opens links as `$BROWSER <url>`.
    if argv.len() == 2 && (argv[1].starts_with("https://") || argv[1].starts_with("http://")) {
        return finish(link::open(&argv[1]).map(|()| 0));
    }
    // Output that may reach an agent goes through the redactor (see privacy.rs). Commands that hand the terminal
    // to the agent or a screen are left alone; they print no identifiers themselves.
    let passthrough = matches!(
        argv.get(1).map(String::as_str),
        Some("run" | "login" | "tui" | "shell-init" | "completions" | "url")
    );
    if !passthrough
        && std::env::var_os("SEATWISE_FILTERED").is_none()
        && seatwise::privacy::active()
    {
        let cfg = Config::load().unwrap_or_default();
        return match seatwise::privacy::filtered(&cfg) {
            Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            Err(e) => {
                eprintln!("seat: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match Cli::parse().cmd {
        Cmd::Hook(h) => hook(h),
        cmd => finish(run(cmd)),
    }
}

fn finish(result: Result<i32>) -> ExitCode {
    match result {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("seat: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Exit 2 blocks the prompt / tool call and shows the message; a warning is JSON on stdout.
/// Fails closed: an unreadable config blocks too (the hook is only installed when enforcement is on).
fn hook(h: HookCmd) -> ExitCode {
    let (seat_name, prompt) = match h {
        HookCmd::Prompt { seat } => (seat, true),
        HookCmd::Tool { seat } => (seat, false),
    };
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let verdict = (|| -> Result<Verdict, String> {
        if guard::forced() {
            return Ok(Verdict::Allow);
        }
        let cfg = Config::load().map_err(|e| format!("{e:#}"))?;
        let seat = cfg.seat(&seat_name).ok_or_else(|| {
            format!(
                "seat '{seat_name}' is not in {}",
                paths::config_file().display()
            )
        })?;
        let input: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("unreadable hook input: {e}"))?;
        Ok(if prompt {
            guard::hook_prompt(&cfg, seat, &input)
        } else {
            guard::hook_tool(&cfg, seat, &input)
        })
    })()
    .unwrap_or_else(Verdict::Block);
    match verdict {
        Verdict::Allow => ExitCode::SUCCESS,
        Verdict::Warn(msg) => {
            let msg = format!("seatwise: {msg}");
            let out = if prompt {
                serde_json::json!({ "systemMessage": msg })
            } else {
                // Ask the user before the tool call goes ahead.
                serde_json::json!({ "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "ask",
                    "permissionDecisionReason": msg,
                }})
            };
            println!("{out}");
            ExitCode::SUCCESS
        }
        Verdict::Block(msg) => {
            eprintln!("seatwise: {msg}");
            ExitCode::from(2)
        }
    }
}

fn pick_seats<'a>(cfg: &'a Config, names: &[String]) -> Result<Vec<&'a Seat>> {
    if names.is_empty() {
        return Ok(cfg.seats.iter().collect());
    }
    names
        .iter()
        .map(|s| cfg.seat(s).with_context(|| format!("unknown seat '{s}'")))
        .collect()
}

/// The seat for a folder: its owner, else the session's seat, else default_seat.
fn seat_here<'a>(cfg: &'a Config, dir: &std::path::Path) -> Option<&'a Seat> {
    match folders::owner_of(cfg, dir) {
        Some(Owner::Seat(s)) => cfg.seat(s),
        _ => link::current_seat(cfg)
            .or_else(|| cfg.default_seat.as_deref().and_then(|d| cfg.seat(d))),
    }
}

/// `seat enable/disable <target>`.
fn toggle(target: &str, on: bool) -> Result<String> {
    let cfg = Config::load()?;
    let (kind, name) = match target.split_once(':') {
        Some((k, n)) => (Some(k), n),
        None => (None, target),
    };
    let mut kinds = Vec::new();
    if kind.is_none_or(|k| k == "seat") && cfg.seat(name).is_some() {
        kinds.push("seat");
    }
    if kind.is_none_or(|k| k == "agent") && (name == "claude" || name == "codex") {
        kinds.push("agent");
    }
    if kind.is_none_or(|k| k == "preset") && seatwise::preset::get(&cfg, name).is_some() {
        kinds.push("preset");
    }
    if kind.is_none_or(|k| k == "group") && cfg.groups.iter().any(|g| g.name == name) {
        kinds.push("group");
    }
    if kind.is_none() && name == "sentinel" {
        kinds.push("sentinel");
    }
    let kind = match kinds.as_slice() {
        [k] => *k,
        [] => bail!(
            "nothing named '{target}': use a seat, agent:claude|codex, preset:<name>, group:<name> or sentinel"
        ),
        many => bail!(
            "'{name}' is a {}; say which, e.g. {}:{name}",
            many.join(" and a "),
            many[0]
        ),
    };
    let mut file = ConfigFile::open()?;
    let default_on = |on: bool| (!on).then(|| toml_edit::Value::from(false));
    match kind {
        "seat" => file.set_seat_enabled(&cfg.seat(name).expect("checked").name, on)?,
        "agent" => file.set(&["agents"], name, default_on(on))?,
        "preset" => file.set_preset_enabled(&cfg, name, on)?,
        "group" => file.set_group(name, "enabled", default_on(on))?,
        _ => file.set(&["sentinel"], "enabled", Some(on.into()))?,
    }
    file.save()?;
    let state = if on { "enabled" } else { "disabled" };
    Ok(if kind == "sentinel" {
        format!("sentinel {state}")
    } else {
        format!("{kind} '{name}' {state}")
    })
}

fn ask(question: &str) -> Result<String> {
    eprint!("{question} ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn here_or(path: Option<PathBuf>) -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    Ok(folders::normalize(
        &path.unwrap_or_else(|| cwd.clone()),
        &cwd,
    ))
}

/// Pin `seat` to the account it is signed into. Returns a description of what was pinned.
fn pin(seat: &Seat) -> Result<String> {
    let account = claude::read_account(seat).with_context(|| {
        format!(
            "seat '{}' is not signed in (`seat login {}`)",
            seat.name, seat.name
        )
    })?;
    let mut file = ConfigFile::open()?;
    let plan = Plan::of(&account);
    file.pin(
        &seat.name,
        account.org_uuid.as_deref(),
        account.org_name.as_deref(),
        account.email.as_deref(),
        plan.as_ref().map(Plan::key).as_deref(),
    )?;
    file.save()?;
    Ok(format!(
        "seat '{}' pinned to {} in {}",
        seat.name,
        [
            account.email,
            account.org_name.or(account.org_uuid),
            plan.map(|p| p.label()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", "),
        paths::config_file().display()
    ))
}

/// After a login: suggest the usage profile that fits the seat's plan, and apply it if the user agrees.
fn offer_profile(cfg: &Config, seat: &Seat) -> Result<()> {
    if seat.agent != seatwise::config::Agent::Claude {
        return Ok(());
    }
    let Some(plan) = claude::read_account(seat).as_ref().and_then(Plan::of) else {
        return Ok(());
    };
    let s = plan::suggest(&plan, None);
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !terminal {
        println!(
            "suggested usage profile: {} ({}); apply with `seat profile {} {}`",
            s.profile, s.why, seat.name, s.profile
        );
        return Ok(());
    }
    print!(
        "Apply the '{}' usage profile to seat '{}' ({})? [y/N] ",
        s.profile, seat.name, s.why
    );
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("y") {
        plan::apply(cfg, seat, &s.profile)?;
        let mut file = ConfigFile::open()?;
        file.set_seat(&seat.name, "profile", &s.profile)?;
        file.save()?;
        println!("profile '{}' applied", s.profile);
    }
    Ok(())
}

fn schedule_out(
    s: &seatwise::sentinel::Schedule,
    dry_run: bool,
    install: bool,
    every: &str,
) -> Result<i32> {
    if dry_run {
        for (path, text) in &s.files {
            if text.is_empty() {
                println!("remove {}", path.display());
            } else {
                println!("write {}:\n{text}", path.display());
            }
        }
        for c in &s.commands {
            println!("run: {}", c.join(" "));
        }
        return Ok(0);
    }
    seatwise::sentinel::carry_out(s, install)?;
    if install {
        println!("sentinel scheduled every {every}; findings show as notifications");
    } else {
        println!("sentinel schedule removed");
    }
    Ok(0)
}

fn run(cmd: Cmd) -> Result<i32> {
    let exe = std::env::current_exe()?;
    match cmd {
        Cmd::Run {
            with,
            home,
            seat,
            args,
        } => launch::run(
            &Config::load()?,
            &seat,
            &launch::RunOptions { with, home },
            &args,
        ),
        Cmd::Desktop { seat } => {
            let cfg = Config::load()?;
            let lines = match seat {
                None => seatwise::desktop::status(&cfg, &exe),
                Some(s) => {
                    let seat = cfg
                        .seat(&s)
                        .with_context(|| format!("unknown seat '{s}' ({})", cfg.seat_names()))?;
                    seatwise::desktop::open(&cfg, seat, &exe)?
                }
            };
            for l in lines {
                println!("{l}");
            }
            Ok(0)
        }
        Cmd::Term {
            seat,
            with,
            home,
            dir,
            print,
            args,
        } => {
            let cfg = Config::load()?;
            let seat = match seat {
                Some(s) => cfg
                    .seat(&s)
                    .with_context(|| format!("unknown seat '{s}'"))?
                    .name
                    .clone(),
                None => {
                    link::current_seat(&cfg).map_or_else(|| "auto".to_string(), |s| s.name.clone())
                }
            };
            let dir = match dir {
                Some(d) => d,
                None => std::env::var_os("CLAUDE_PROJECT_DIR")
                    .map(PathBuf::from)
                    .map_or_else(std::env::current_dir, Ok)?,
            };
            let dir = folders::normalize(&dir, &std::env::current_dir()?);
            let with = if with.is_empty() {
                cfg.terminal.profile.iter().cloned().collect()
            } else {
                with
            };
            let words = seatwise::term::run_words(&exe, &seat, &with, home, &args);
            let line = seatwise::term::shell_line(&dir, &words);
            if print {
                println!("{line}");
                return Ok(0);
            }
            seatwise::term::open(&cfg, &dir, &line)?;
            let with_note = if with.is_empty() {
                String::new()
            } else {
                format!(" with {}", with.join(", "))
            };
            println!(
                "opened a terminal: seat {seat}{with_note}, in {}",
                dir.display()
            );
            Ok(0)
        }
        Cmd::Home(HomeCmd::Init { seat, path }) => {
            let cfg = Config::load()?;
            let seat = cfg
                .seat(&seat)
                .with_context(|| format!("unknown seat '{seat}'"))?
                .clone();
            let dir = match path {
                Some(p) => here_or(Some(p))?,
                None => seat
                    .home()
                    .unwrap_or_else(|| seatwise::home::default_path(&seat)),
            };
            if let Some(Owner::Seat(other)) = folders::owner_of(&cfg, &dir)
                && other != seat.name
            {
                bail!("{} belongs to seat '{other}'", dir.display());
            }
            let made = seatwise::home::scaffold(&seat, &dir)?;
            let mut file = ConfigFile::open()?;
            file.set_seat(&seat.name, "home", &paths::contract(&dir))?;
            file.link(&dir, &seat.name);
            file.save()?;
            let cfg = Config::load()?;
            let seat = cfg.seat(&seat.name).expect("still there");
            let agents = seatwise::home::sync(&cfg, seat)?;
            if made.is_empty() {
                println!(
                    "home repo of seat '{}': {} (adopted)",
                    seat.name,
                    dir.display()
                );
            } else {
                println!(
                    "home repo of seat '{}': {} (created {})",
                    seat.name,
                    dir.display(),
                    made.join(", ")
                );
            }
            println!("pointers written to {}", agents.display());
            println!("start there: seat run --home {}", seat.name);
            Ok(0)
        }
        Cmd::Home(HomeCmd::Sync { seats }) => {
            let cfg = Config::load()?;
            let chosen: Vec<&Seat> = if seats.is_empty() {
                cfg.seats.iter().filter(|s| s.home.is_some()).collect()
            } else {
                pick_seats(&cfg, &seats)?
            };
            for seat in chosen {
                let f = seatwise::home::sync(&cfg, seat)?;
                println!(
                    "seat '{}': {} repo(s) listed in {}",
                    seat.name,
                    seatwise::home::repos(&cfg, seat).len(),
                    f.display()
                );
            }
            Ok(0)
        }
        Cmd::Skills(SkillsCmd::Install { seats }) => {
            let cfg = Config::load()?;
            for seat in pick_seats(&cfg, &seats)? {
                let done = seatwise::skills::install(&cfg, seat, &exe)?;
                println!(
                    "seat '{}': skills installed in {}{}",
                    seat.name,
                    paths::contract(&done.dir),
                    seatwise::hybrid::install_note(&done)
                );
                if seat.agent == seatwise::config::Agent::Claude {
                    println!(
                        "  {} (/reload-plugins in a running session)",
                        seatwise::skills::SKILLS
                            .iter()
                            .map(|(n, _)| seatwise::skills::invocation(seat.agent, n))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
            }
            Ok(0)
        }
        Cmd::Skills(SkillsCmd::Show { name }) => {
            let name = name
                .trim_start_matches('/')
                .trim_start_matches("b070nd:")
                .trim_start_matches("seat-");
            let (n, text) = seatwise::skills::SKILLS
                .iter()
                .find(|(n, _)| *n == name)
                .with_context(|| {
                    format!("unknown skill '{name}' (hybrid, terminal, status, home)")
                })?;
            print!(
                "{}",
                seatwise::skills::render(seatwise::config::Agent::Claude, n, text, &exe)
            );
            Ok(0)
        }
        Cmd::Init { print, force } => {
            let found = init::discover();
            if found.is_empty() {
                bail!(
                    "no Claude Code config folders found in your home folder; `seat add <name>` starts one"
                );
            }
            let proposal = init::propose(&found);
            let target = paths::config_file();
            if print || (target.exists() && !force) {
                print!("{proposal}");
                if !print {
                    eprintln!(
                        "seat: {} already exists; this is the proposal (--force replaces it)",
                        target.display()
                    );
                }
                return Ok(0);
            }
            let cfg: Config = toml::from_str(&proposal)?;
            cfg.validate()?;
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&target, proposal)?;
            println!("wrote {}", target.display());
            print!("{}", status::list(&cfg, &exe));
            println!(
                "next: review it, choose enforcement (\"warn\" recommended), then `seat doctor`"
            );
            Ok(0)
        }
        Cmd::Add {
            name,
            config_dir,
            aliases,
            from,
            agent,
        } => {
            let agent = match agent.as_str() {
                "claude" => seatwise::config::Agent::Claude,
                "codex" => seatwise::config::Agent::Codex,
                a => bail!("unknown agent '{a}' (claude or codex)"),
            };
            let dir = config_dir.unwrap_or_else(|| format!("~/{}-{name}", agent.default_dir()));
            let mut file = ConfigFile::open()?;
            file.add_seat(&name, &dir, &aliases)?;
            if agent != seatwise::config::Agent::Claude {
                file.set_seat(&name, "agent", agent.as_str())?;
            }
            let cfg = file.config()?;
            let source = match &from {
                Some(f) => Some(
                    cfg.seat(f)
                        .with_context(|| format!("unknown seat '{f}'"))?
                        .clone(),
                ),
                None => None,
            };
            file.save()?;
            let seat = cfg.seat(&name).expect("just added");
            let existed = seat.config_dir().is_dir();
            std::fs::create_dir_all(seat.config_dir())?;
            println!(
                "seat '{name}' added ({} {})",
                if existed { "adopted" } else { "created" },
                seat.config_dir().display()
            );
            if let Some(src) = source {
                let copied = share::copy_setup(&src, seat)?;
                println!("copied from '{}': {}", src.name, copied.join(", "));
            }
            if claude::install_hooks(&cfg, seat, &exe)? {
                println!(
                    "hooks installed (enforcement: {})",
                    cfg.enforcement_of(seat).as_str()
                );
            }
            if claude::read_account(seat).is_none() {
                println!(
                    "next: `seat login {name}`, then `seat link {name}` in its project folders"
                );
            }
            Ok(0)
        }
        Cmd::Login { seat } => {
            let cfg = Config::load()?;
            let seat = cfg
                .seat(&seat)
                .with_context(|| format!("unknown seat '{seat}'"))?;
            let code = launch::login(seat)?;
            if code == 0
                && seat.org_uuid.is_none()
                && seat.email.is_none()
                && let Ok(msg) = pin(seat)
            {
                println!("{msg}");
            }
            if code == 0 && seat.profile.is_none() {
                offer_profile(&cfg, seat)?;
            }
            Ok(code)
        }
        Cmd::Pin { seat } => {
            let cfg = Config::load()?;
            let seat = cfg
                .seat(&seat)
                .with_context(|| format!("unknown seat '{seat}'"))?;
            println!("{}", pin(seat)?);
            Ok(0)
        }
        Cmd::Profile { seat, name, usage } => {
            let cfg = Config::load()?;
            let seat = cfg
                .seat(&seat)
                .with_context(|| format!("unknown seat '{seat}'"))?;
            match name {
                Some(name) => {
                    plan::apply(&cfg, seat, &name)?;
                    let mut file = ConfigFile::open()?;
                    file.set_seat(&seat.name, "profile", &name)?;
                    file.save()?;
                    println!(
                        "seat '{}': profile '{name}' applied to {}",
                        seat.name,
                        seat.config_dir().join("settings.json").display()
                    );
                }
                None => {
                    let account = claude::read_account(seat)
                        .with_context(|| format!("seat '{}' is not signed in", seat.name))?;
                    let plan = Plan::of(&account).with_context(|| {
                        format!("seat '{}': the account file names no plan", seat.name)
                    })?;
                    let live = if usage {
                        Some(usage::fetch(seat)?)
                    } else {
                        None
                    };
                    println!("seat '{}': {} plan", seat.name, plan.label());
                    if seat.agent != seatwise::config::Agent::Claude {
                        println!(
                            "  usage profiles are Claude Code settings; Codex has its own in config.toml"
                        );
                        return Ok(0);
                    }
                    let s = plan::suggest(&plan, live.as_ref());
                    if account.extra_usage {
                        println!("  extra usage is on: use beyond the plan is billed");
                    }
                    if let Some(u) = &live {
                        for line in usage::render(u) {
                            println!("  {line}");
                        }
                    }
                    println!(
                        "  current profile: {}",
                        seat.profile.as_deref().unwrap_or("none")
                    );
                    println!("  suggested: {} ({})", s.profile, s.why);
                    println!("profiles:");
                    for (n, d) in plan::all(&cfg) {
                        println!("  {n:<9} {d}");
                    }
                    println!("apply: seat profile {} {}", seat.name, s.profile);
                }
            }
            Ok(0)
        }
        Cmd::Usage { seats } => {
            let cfg = Config::load()?;
            let mut failed = false;
            for seat in pick_seats(&cfg, &seats)? {
                match usage::fetch(seat) {
                    Ok(u) if u.windows.is_empty() => println!("{}: no limits reported", seat.name),
                    Ok(u) => {
                        println!("{}", seat.name);
                        for line in usage::render(&u) {
                            println!("  {line}");
                        }
                    }
                    Err(e) => {
                        failed = true;
                        println!("{}: {e:#}", seat.name);
                    }
                }
            }
            Ok(i32::from(failed))
        }
        Cmd::Link {
            from: Some(file), ..
        } => {
            let cfg = Config::load()?;
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let mut entries = Vec::new();
            let mut errors = Vec::new();
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((owner, path)) = line.split_once(char::is_whitespace) else {
                    errors.push(format!("line {}: expected `<owner> <path>`", n + 1));
                    continue;
                };
                let owner = match owner {
                    seatwise::config::ANY | seatwise::config::NONE => owner.to_string(),
                    s => match cfg.seat(s) {
                        Some(seat) => seat.name.clone(),
                        None => {
                            errors.push(format!("line {}: unknown seat '{s}'", n + 1));
                            continue;
                        }
                    },
                };
                let path = seatwise::paths::expand(path.trim());
                if !path.is_absolute() {
                    errors.push(format!(
                        "line {}: {} is not an absolute path (or ~/...)",
                        n + 1,
                        path.display()
                    ));
                    continue;
                }
                entries.push((owner, path));
            }
            if !errors.is_empty() {
                bail!("nothing written:\n  {}", errors.join("\n  "));
            }
            let mut file = ConfigFile::open()?;
            for (owner, path) in &entries {
                file.link(path, owner);
            }
            file.save()?;
            for (owner, path) in &entries {
                println!("{}: {owner}", path.display());
            }
            Ok(0)
        }
        Cmd::Link { owner, path, .. } => {
            let cfg = Config::load()?;
            let owner = owner.expect("required without --from");
            let owner = match owner.as_str() {
                seatwise::config::ANY | seatwise::config::NONE => owner,
                s => cfg
                    .seat(s)
                    .with_context(|| format!("unknown seat '{s}' (or use \"any\" / \"none\")"))?
                    .name
                    .clone(),
            };
            let path = here_or(path)?;
            let mut file = ConfigFile::open()?;
            file.link(&path, &owner);
            file.save()?;
            println!("{}: {owner}", path.display());
            Ok(0)
        }
        Cmd::Unlink { missing: true, .. } => {
            let mut file = ConfigFile::open()?;
            let removed = file.unlink_missing();
            if removed.is_empty() {
                println!("every folder entry exists");
                return Ok(0);
            }
            file.save()?;
            for p in removed {
                println!("{p}: entry removed (folder doesn't exist)");
            }
            Ok(0)
        }
        Cmd::Unlink { path, .. } => {
            let path = here_or(path)?;
            let mut file = ConfigFile::open()?;
            if !file.unlink(&path) {
                bail!(
                    "{} has no entry of its own (see `seat owner`)",
                    path.display()
                );
            }
            file.save()?;
            println!("{}: entry removed", path.display());
            Ok(0)
        }
        Cmd::Owner { path } => {
            let cfg = Config::load()?;
            let p = here_or(path)?;
            let owner = match folders::owner_of(&cfg, &p) {
                Some(Owner::Seat(s)) => s.to_string(),
                Some(Owner::Any) => "any (shared start folder)".into(),
                Some(Owner::Nobody) => "none (closed to every seat)".into(),
                None => "unassigned".into(),
            };
            println!("{}: {owner}", p.display());
            Ok(0)
        }
        Cmd::List => {
            print!("{}", status::list(&Config::load()?, &exe));
            Ok(0)
        }
        Cmd::Show { seat, json } => {
            let cfg = Config::load()?;
            let s = cfg
                .seat(&seat)
                .with_context(|| format!("unknown seat '{seat}' (`seat list`)"))?;
            let v = seatwise::view::seat(&cfg, s, &exe, &seatwise::desktop::running(&cfg));
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", seatwise::view::seat_text(&v));
            }
            Ok(0)
        }
        Cmd::Projects { json } => {
            let cfg = Config::load()?;
            let p = seatwise::view::projects(&cfg);
            if json {
                println!("{}", serde_json::to_string_pretty(&p)?);
            } else if p.is_empty() {
                println!("no project folders yet: `seat link <seat> <folder>`");
            } else {
                print!("{}", seatwise::view::projects_text(&p, true));
            }
            Ok(0)
        }
        Cmd::Ps => {
            // Works before there is a config too: sessions then show their config folder.
            let cfg = if paths::config_file().exists() {
                Config::load()?
            } else {
                Config::default()
            };
            print!("{}", ps::list(&cfg)?);
            Ok(0)
        }
        Cmd::Doctor { fix } => {
            let cfg = Config::load()?;
            let mut findings = doctor::check(&cfg, &exe);
            if fix {
                for done in doctor::fix(&cfg, &exe, &findings)? {
                    println!("fixed: {done}");
                }
                findings = doctor::check(&cfg, &exe);
            }
            print!("{}", doctor::report(&findings));
            Ok(i32::from(
                findings.iter().any(|f| f.level == doctor::Level::Error),
            ))
        }
        Cmd::InstallHooks { seats } => {
            let cfg = Config::load()?;
            for seat in pick_seats(&cfg, &seats)? {
                let file = seat.config_dir().join("settings.json");
                if claude::install_hooks(&cfg, seat, &exe)? {
                    println!(
                        "seat '{}': hooks installed in {} (enforcement: {})",
                        seat.name,
                        file.display(),
                        cfg.enforcement_of(seat).as_str()
                    );
                } else {
                    println!(
                        "seat '{}': enforcement is off, no seatwise hooks in {}",
                        seat.name,
                        file.display()
                    );
                }
            }
            Ok(0)
        }
        Cmd::Share { seats } => {
            let cfg = Config::load()?;
            for seat in pick_seats(&cfg, &seats)? {
                let notes = share::apply(&cfg, seat)?;
                if notes.is_empty() {
                    println!("seat '{}': shared items up to date", seat.name);
                }
                for n in notes {
                    println!("{n}");
                }
            }
            Ok(0)
        }
        Cmd::ShellInit { shell } => {
            print!("{}", shell::init(shell, &Config::load()?, &exe));
            Ok(0)
        }
        Cmd::Shim(ShimCmd::Install) => {
            let f = shim::install(&exe)?;
            println!("wrote {}", f.display());
            if !shim::first_on_path() {
                println!(
                    "put {} first on PATH, e.g. export PATH=\"{}:$PATH\"",
                    shim::dir().display(),
                    shim::dir().display()
                );
            }
            Ok(0)
        }
        Cmd::Shim(ShimCmd::Uninstall) => {
            println!(
                "{}",
                if shim::uninstall()? {
                    "shim removed"
                } else {
                    "no shim installed"
                }
            );
            Ok(0)
        }
        Cmd::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "seat", &mut std::io::stdout());
            Ok(0)
        }
        Cmd::Sync(SyncCmd::Restore { seat }) => {
            let cfg = Config::load()?;
            let seat = cfg
                .seat(&seat)
                .with_context(|| format!("unknown seat '{seat}'"))?;
            for note in sync::restore(seat, &cfg.track)? {
                println!("{note}");
            }
            Ok(0)
        }
        Cmd::ConfigPath => {
            println!("{}", paths::config_file().display());
            Ok(0)
        }
        Cmd::Hybrid {
            seat,
            dir,
            with,
            preset,
            link,
            check,
            no_open,
            print,
            json,
        } => {
            let opts = seatwise::hybrid::Options {
                seat,
                dir,
                with,
                preset,
                link,
                check,
                no_open,
                print,
            };
            if json {
                let cfg = Config::load()?;
                let plan = seatwise::hybrid::plan(&cfg, &exe, &opts)?;
                println!("{}", serde_json::to_string_pretty(&plan)?);
                return Ok(0);
            }
            for line in seatwise::hybrid::run(&exe, &opts)? {
                println!("{line}");
            }
            Ok(0)
        }
        Cmd::Enable { target } => {
            println!("{}", toggle(&target, true)?);
            Ok(0)
        }
        Cmd::Disable { target } => {
            println!("{}", toggle(&target, false)?);
            Ok(0)
        }
        Cmd::Explain { path, seat, with } => {
            let cfg = Config::load()?;
            let dir = here_or(path)?;
            let seat = match seat {
                Some(s) => cfg
                    .seat(&s)
                    .with_context(|| format!("unknown seat '{s}'"))?,
                None => seat_here(&cfg, &dir)
                    .with_context(|| format!("no seat for {}; pass --seat", dir.display()))?,
            };
            print!("{}", seatwise::preset::explain(&cfg, seat, &dir, &with));
            Ok(0)
        }
        Cmd::Preset(PresetCmd::List) => {
            let cfg = Config::load()?;
            let mut rows = vec![
                ["PRESET", "ON", "PROFILES", "USED BY", "WHAT IT IS FOR"]
                    .map(String::from)
                    .to_vec(),
            ];
            for p in seatwise::preset::all(&cfg) {
                let used: Vec<&str> = cfg
                    .groups
                    .iter()
                    .filter(|g| g.preset == p.name)
                    .map(|g| g.name.as_str())
                    .collect();
                rows.push(vec![
                    if p.builtin {
                        p.name.clone()
                    } else {
                        format!("{} (yours)", p.name)
                    },
                    if p.enabled { "yes" } else { "no" }.into(),
                    p.profiles.join("+"),
                    if used.is_empty() {
                        "-".into()
                    } else {
                        used.join(", ")
                    },
                    p.description.clone(),
                ]);
            }
            print!("{}", status::table(&rows));
            Ok(0)
        }
        Cmd::Settings(SettingsCmd::Search {
            query,
            experimental,
            env,
            all,
        }) => {
            use seatwise::catalog::{Filter, Kind};
            let hits = seatwise::catalog::search(
                &query.join(" "),
                Filter {
                    experimental,
                    kind: env.then_some(Kind::Env),
                    deprecated: all,
                },
            );
            if hits.is_empty() {
                println!("nothing matches; `seat settings search` lists everything");
                return Ok(1);
            }
            let mut rows = vec![
                ["KEY", "VALUE", "", "WHAT IT DOES"]
                    .map(String::from)
                    .to_vec(),
            ];
            for e in &hits {
                let mark = match e.stability.as_str() {
                    "experimental" => "exp",
                    "deprecated" => "old",
                    _ => "",
                };
                let mut value = e.type_label();
                if value.chars().count() > 28 {
                    value = value.chars().take(27).collect::<String>() + "…";
                }
                rows.push(vec![e.name.clone(), value, mark.into(), e.meaning.clone()]);
            }
            print!("{}", seatwise::status::table(&rows));
            println!(
                "\n{} of them. `seat settings show <key>` for details. Catalog checked {}.",
                hits.len(),
                seatwise::catalog::CHECKED
            );
            Ok(0)
        }
        Cmd::Settings(SettingsCmd::Show { key }) => {
            let Some(e) = seatwise::catalog::lookup(&key) else {
                let mut near = seatwise::catalog::near(&key);
                near.extend(seatwise::catalog::search(&key, Default::default()));
                near.dedup_by(|a, b| a.name == b.name);
                eprintln!("seat: {key} is not in the settings catalog");
                for e in near.iter().take(5) {
                    eprintln!("  did you mean {}?", e.name);
                }
                return Ok(1);
            };
            print!("{}", seatwise::catalog::describe(e));
            Ok(0)
        }
        Cmd::Settings(SettingsCmd::Check { name, file, target }) => {
            use seatwise::catalog::{Level, Target};
            let cfg = Config::load()?;
            let target = match target.as_deref() {
                None | Some("session") => Target::Session,
                Some("local") | Some("project") => Target::Local,
                Some("user") => Target::User,
                Some(other) => anyhow::bail!("unknown target '{other}': session, local or user"),
            };
            let (what, settings) = match (name, file) {
                (_, Some(f)) => (
                    f.display().to_string(),
                    serde_json::from_str(&std::fs::read_to_string(&f)?)
                        .with_context(|| format!("{} is not JSON", f.display()))?,
                ),
                (Some(n), None) => match seatwise::preset::get(&cfg, &n) {
                    Some(p) => (
                        format!("preset '{n}'"),
                        seatwise::preset::settings(&cfg, &p),
                    ),
                    None => (
                        format!("profile '{n}'"),
                        seatwise::plan::settings_of(&cfg, &n)
                            .with_context(|| format!("no preset or profile named '{n}'"))?,
                    ),
                },
                (None, None) => anyhow::bail!("name a preset or profile, or pass --file"),
            };
            let problems = seatwise::catalog::check(&settings, target);
            if problems.is_empty() {
                println!("{what}: no problems for {}", target.label());
                return Ok(0);
            }
            println!("{what}, applied in {}:", target.label());
            print!("{}", seatwise::catalog::report(&problems));
            Ok(if problems.iter().any(|p| p.level == Level::Error) {
                1
            } else {
                0
            })
        }
        Cmd::Preset(PresetCmd::New {
            name,
            from,
            description,
            interactive,
        }) => {
            let cfg = Config::load()?;
            if cfg.presets.contains_key(&name) {
                anyhow::bail!("preset '{name}' exists; `seat preset edit {name}` changes it");
            }
            let mut d = match from.as_deref() {
                None => seatwise::draft::Draft::new(&name),
                Some(f) if seatwise::preset::get(&cfg, f).is_some() => {
                    seatwise::draft::Draft::from_preset(&cfg, f, &name)?
                }
                Some(f) => seatwise::draft::Draft::from_profile(&cfg, f, &name)?,
            };
            if let Some(text) = description {
                d.description = text;
            }
            preset_session(&cfg, d, interactive)
        }
        Cmd::Preset(PresetCmd::Edit { name }) => {
            let cfg = Config::load()?;
            let d = seatwise::draft::Draft::from_preset(&cfg, &name, &name)?;
            preset_session(&cfg, d, true)
        }
        Cmd::Preset(PresetCmd::Set { name, key, value }) => {
            let cfg = Config::load()?;
            let mut d = draft_of(&cfg, &name)?;
            let notes = d.set(&cfg, &key, &value)?;
            let mut file = ConfigFile::open()?;
            d.save(&cfg, &mut file)?;
            file.save()?;
            println!("preset '{name}': {key} = {value}");
            for n in notes {
                println!("note: {}", n.message);
            }
            Ok(0)
        }
        Cmd::Preset(PresetCmd::Unset { name, key }) => {
            let cfg = Config::load()?;
            let mut d = draft_of(&cfg, &name)?;
            if !d.unset(&key) {
                println!("preset '{name}' doesn't set {key}");
                return Ok(1);
            }
            let mut file = ConfigFile::open()?;
            d.save(&cfg, &mut file)?;
            file.save()?;
            println!("preset '{name}': {key} removed");
            Ok(0)
        }
        Cmd::Preset(PresetCmd::Profiles { name, profiles }) => {
            let cfg = Config::load()?;
            let mut d = draft_of(&cfg, &name)?;
            d.set_profiles(&cfg, &profiles)?;
            let mut file = ConfigFile::open()?;
            d.save(&cfg, &mut file)?;
            file.save()?;
            println!(
                "preset '{name}': profiles {}",
                if profiles.is_empty() {
                    "-".into()
                } else {
                    profiles.join(" + ")
                }
            );
            Ok(0)
        }
        Cmd::Preset(PresetCmd::Options {
            name,
            query,
            experimental,
        }) => {
            let cfg = Config::load()?;
            let d = draft_of(&cfg, &name).unwrap_or_else(|_| seatwise::draft::Draft::new(&name));
            let options = d.options(&cfg, &query.join(" "), experimental);
            println!(
                "preset '{name}' is applied in {}; these settings can go in it:",
                d.target.label()
            );
            for o in &options {
                let now = o
                    .current
                    .as_ref()
                    .map(|v| format!("  [now {v}]"))
                    .unwrap_or_default();
                let exp = if o.entry.experimental() {
                    "  (experimental)"
                } else {
                    ""
                };
                let values = if o.values.is_empty() {
                    o.entry.type_label()
                } else {
                    o.values.join(" | ")
                };
                println!(
                    "{}{exp}{now}\n    {}\n    values: {values}",
                    o.entry.name, o.entry.meaning
                );
                if let Some(b) = &o.blocked {
                    println!("    not now: {b}");
                }
            }
            println!(
                "{} setting(s). `seat preset set {name} <key> <value>` sets one.",
                options.len()
            );
            Ok(0)
        }
        Cmd::Preset(PresetCmd::Show { name }) => {
            let cfg = Config::load()?;
            let p = seatwise::preset::get(&cfg, &name)
                .with_context(|| format!("unknown preset '{name}'"))?;
            println!("{}: {}", p.name, p.description);
            println!(
                "{}",
                serde_json::to_string_pretty(&seatwise::preset::settings(&cfg, &p))?
            );
            Ok(0)
        }
        Cmd::Group(GroupCmd::List) => {
            let cfg = Config::load()?;
            let mut rows = vec![
                ["GROUP", "ON", "PRESET", "APPLY", "SOURCES", "PATHS"]
                    .map(String::from)
                    .to_vec(),
            ];
            for g in &cfg.groups {
                rows.push(vec![
                    g.name.clone(),
                    if g.enabled { "yes" } else { "no" }.into(),
                    g.preset.clone(),
                    g.apply.unwrap_or(cfg.preset_apply).as_str().into(),
                    g.sources.as_ref().map_or("all".into(), |s| s.join(",")),
                    g.paths.join(", "),
                ]);
            }
            if cfg.groups.is_empty() {
                println!("no folder groups: `seat group add` makes one");
            } else {
                print!("{}", status::table(&rows));
            }
            Ok(0)
        }
        Cmd::Group(GroupCmd::Add {
            path,
            name,
            preset,
            apply,
            sources,
        }) => {
            let cfg = Config::load()?;
            let dir = here_or(path)?;
            let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
            let name = name.unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "group".into())
            });
            let preset = match preset {
                Some(p) => p,
                None if terminal => {
                    let all = seatwise::preset::all(&cfg);
                    eprintln!("Preset for {}:", dir.display());
                    for (i, p) in all.iter().enumerate() {
                        eprintln!("  {}) {:<9} {}", i + 1, p.name, p.description);
                    }
                    eprintln!("  0) none      no preset here");
                    let a = ask(">")?;
                    match a.parse::<usize>() {
                        Ok(0) => "none".into(),
                        Ok(i) => all
                            .get(i - 1)
                            .map(|p| p.name.clone())
                            .context("no such preset")?,
                        Err(_) => a,
                    }
                }
                None => bail!("--preset is needed (see `seat preset list`)"),
            };
            let apply = match apply.as_deref() {
                Some("session") => Some(seatwise::config::Apply::Session),
                Some("project") => Some(seatwise::config::Apply::Project),
                Some(other) => bail!("--apply must be session or project, not {other}"),
                None if terminal && preset != "none" => {
                    let a = ask(&format!(
                        "Apply it per session (passed at start, nothing written) or into the project's .claude/settings.local.json? [session/project, default {}]",
                        cfg.preset_apply.as_str()
                    ))?;
                    match a.as_str() {
                        "project" => Some(seatwise::config::Apply::Project),
                        "session" => Some(seatwise::config::Apply::Session),
                        _ => None,
                    }
                }
                None => None,
            };
            let group = seatwise::config::Group {
                name: name.clone(),
                paths: vec![paths::contract(&dir)],
                preset,
                apply,
                sources: sources.map(|s| s.split(',').map(|x| x.trim().to_string()).collect()),
                enabled: true,
            };
            let mut file = ConfigFile::open()?;
            file.add_group(&group)?;
            file.save()?;
            println!(
                "group '{name}': {} → preset {}",
                paths::contract(&dir),
                group.preset
            );
            println!("check it: seat explain {}", paths::contract(&dir));
            Ok(0)
        }
        Cmd::Group(GroupCmd::Remove { name }) => {
            let mut file = ConfigFile::open()?;
            if !file.remove_group(&name) {
                bail!("no group '{name}'");
            }
            file.save()?;
            println!("group '{name}' removed");
            Ok(0)
        }
        Cmd::Sentinel(args) => {
            let cfg = Config::load()?;
            match args.cmd {
                Some(SentinelCmd::Rules) => {
                    for (rule, what) in seatwise::sentinel::RULES {
                        println!("{rule:<18} {what}");
                    }
                    return Ok(0);
                }
                Some(SentinelCmd::Install { dry_run }) => {
                    let s = seatwise::sentinel::schedule(&cfg, &exe)?;
                    return schedule_out(&s, dry_run, true, &cfg.sentinel.interval);
                }
                Some(SentinelCmd::Uninstall { dry_run }) => {
                    let s = seatwise::sentinel::unschedule();
                    return schedule_out(&s, dry_run, false, &cfg.sentinel.interval);
                }
                None => {}
            }
            let findings = if args.here {
                let dir = here_or(None)?;
                let seat = seat_here(&cfg, &dir)
                    .with_context(|| format!("no seat for {}", dir.display()))?;
                seatwise::sentinel::check_here(&cfg, seat, &dir)
            } else {
                let only = if args.seats.is_empty() {
                    vec![]
                } else {
                    pick_seats(&cfg, &args.seats)?
                };
                seatwise::sentinel::run(&cfg, &only)
            };
            if args.notify {
                seatwise::sentinel::notify(&findings);
            }
            if !args.quiet {
                if findings.is_empty() {
                    println!("no suggestions");
                }
                for f in &findings {
                    println!("{}", f.line());
                }
            }
            Ok(0)
        }
        Cmd::Tui => seatwise::tui::run(&exe).map(|()| 0),
        Cmd::Url { url, copy } => {
            if copy {
                let cfg = Config::load().ok();
                let seat = cfg.as_ref().and_then(link::current_seat);
                link::copy(&url, seat.map(|s| s.name.as_str())).map(|()| 0)
            } else {
                link::open(&url).map(|()| 0)
            }
        }
        Cmd::Hook(_) => unreachable!("handled in main"),
    }
}
