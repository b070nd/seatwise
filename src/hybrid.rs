//! `seat hybrid`: set up the command-line side of a project next to the desktop app, in one step.
//!
//! From a desktop (or CLI) session it works out the session's seat and project, then makes sure the CLI side is
//! ready: the folder belongs to that seat, the seat has the seatwise skills, the project has the use-case preset
//! you want. Then it opens a terminal on the same seat and project. Each step reports what it found or did.
//! `--check` only reports.

use crate::config::{Config, Owner, Seat};
use crate::edit::ConfigFile;
use crate::{folders, link, paths, preset, skills, term};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct Options {
    pub seat: Option<String>,
    pub dir: Option<PathBuf>,
    pub with: Vec<String>,
    /// Give the project this preset (adds or updates its folder group).
    pub preset: Option<String>,
    /// Give an unassigned project folder to the seat.
    pub link: bool,
    /// Only report; change nothing, open nothing.
    pub check: bool,
    /// Set up, but don't open a terminal.
    pub no_open: bool,
    /// Print the terminal's command instead of opening it.
    pub print: bool,
}

fn project_dir(opts: &Options) -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let dir = match &opts.dir {
        Some(d) => d.clone(),
        None => std::env::var_os("CLAUDE_PROJECT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.clone()),
    };
    Ok(preset::project_root(&folders::normalize(&dir, &cwd)))
}

fn pick_seat<'a>(cfg: &'a Config, opts: &Options, root: &Path) -> Result<&'a Seat> {
    if let Some(s) = &opts.seat {
        return cfg.seat(s).with_context(|| format!("unknown seat '{s}'"));
    }
    if let Some(s) = link::current_seat(cfg) {
        return Ok(s);
    }
    if let Some(Owner::Seat(s)) = folders::owner_of(cfg, root) {
        return Ok(cfg.seat(s).expect("validated config"));
    }
    if let Some(s) = cfg.default_seat.as_deref().and_then(|d| cfg.seat(d)) {
        return Ok(s);
    }
    bail!(
        "can't tell which seat this is for; pass --seat ({})",
        cfg.seat_names()
    )
}

/// A group name not used yet, from the folder's name.
fn group_name(cfg: &Config, root: &Path) -> String {
    let base = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let mut name = base.clone();
    let mut n = 2;
    while cfg.groups.iter().any(|g| g.name == name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Run it. Returns the report, one line per step.
pub fn run(exe: &Path, opts: &Options) -> Result<Vec<String>> {
    let mut cfg = Config::load()?;
    let root = project_dir(opts)?;
    let seat = pick_seat(&cfg, opts, &root)?.clone();
    if !cfg.is_enabled(&seat) {
        bail!(
            "seat '{}' is disabled (`seat enable {}`)",
            seat.name,
            seat.name
        );
    }
    let n = &seat.name;
    let shown = paths::contract(&root);
    let mut report = vec![format!("seat     {n} ({})", seat.agent.as_str())];
    report.push(format!("project  {shown}"));

    // 1. The folder belongs to this seat.
    match folders::owner_of(&cfg, &root) {
        Some(Owner::Seat(s)) if s == seat.name => report.push(format!("owner    {n}: ok")),
        Some(Owner::Seat(other)) => bail!(
            "{shown} belongs to seat '{other}', not '{n}'; open the terminal from a '{other}' session"
        ),
        Some(Owner::Nobody) => bail!("{shown} is closed to every seat (owner \"none\")"),
        Some(Owner::Any) => report.push("owner    shared start folder: any seat may start here".into()),
        None if opts.link && !opts.check => {
            let mut file = ConfigFile::open()?;
            file.link(&root, n);
            file.save()?;
            cfg = Config::load()?;
            report.push(format!("owner    {shown} now belongs to {n}"));
        }
        None => report.push(format!(
            "owner    not assigned: `seat hybrid --link` gives it to {n}, so `seat run auto` and the guard know it"
        )),
    }

    // 2. The seat has the seatwise skills.
    let have = skills::state(&seat, exe);
    if have.current() {
        report.push("skills   up to date".into());
    } else if opts.check {
        report.push(format!("skills   {}", have.summary()));
    } else {
        let done = skills::install(&cfg, &seat, exe)?;
        report.push(format!(
            "skills   installed in {}{}",
            paths::contract(&done.dir),
            install_note(&done)
        ));
    }

    // 3. The project's preset.
    if let Some(p) = &opts.preset {
        if p != crate::config::NONE && preset::get(&cfg, p).is_none() {
            bail!("unknown preset '{p}' (`seat preset list`)");
        }
        let key = folders::key_string(&root);
        let exact = cfg
            .groups
            .iter()
            .find(|g| {
                g.paths
                    .iter()
                    .any(|x| folders::key_string(&paths::expand(x)) == key)
            })
            .map(|g| g.name.clone());
        if opts.check {
            report.push(format!("preset   would become {p}"));
        } else {
            let mut file = ConfigFile::open()?;
            let name = match exact {
                Some(name) => {
                    file.set_group(&name, "preset", Some(p.as_str().into()))?;
                    name
                }
                None => {
                    let name = group_name(&cfg, &root);
                    file.add_group(&crate::config::Group {
                        name: name.clone(),
                        paths: vec![shown.clone()],
                        preset: p.clone(),
                        apply: None,
                        sources: None,
                        enabled: true,
                    })?;
                    name
                }
            };
            file.save()?;
            cfg = Config::load()?;
            report.push(format!("preset   {p} (group '{name}')"));
        }
    } else {
        let r = preset::resolve(&cfg, &seat, &root);
        report.push(match (&r.preset, &r.group) {
            (Some(p), Some(g)) => format!(
                "preset   {p} from group '{g}', applied per {}",
                r.apply.as_str()
            ),
            (None, Some(g)) => format!("preset   none (group '{g}')"),
            _ => "preset   none: `--preset explore|build|review|quick` sets one for this project"
                .into(),
        });
    }

    // 4. The terminal.
    let with = if opts.with.is_empty() {
        cfg.terminal.profile.iter().cloned().collect()
    } else {
        opts.with.clone()
    };
    let words = term::run_words(exe, &seat.name, &with, false, &[]);
    let line = term::shell_line(&root, &words);
    let with_note = if with.is_empty() {
        String::new()
    } else {
        format!(", with {}", with.join(", "))
    };
    if opts.check || opts.no_open {
        report.push(format!(
            "terminal not opened (`seat hybrid` opens it{with_note})"
        ));
    } else if opts.print {
        report.push(format!("terminal {line}"));
    } else {
        term::open(&cfg, &root, &line)?;
        report.push(format!("terminal opened on {n}{with_note}"));
    }
    Ok(report)
}

/// What to add after "installed in <dir>": registration and removed old skills.
pub fn install_note(done: &skills::Installed) -> String {
    let mut out = String::new();
    if !done.removed.is_empty() {
        out += &format!("; removed {} old loose skill(s)", done.removed.len());
    }
    match &done.note {
        Some(why) => out += &format!("; the plugin registers when the next session starts ({why})"),
        None if !done.registered => out += "; the plugin registers when the next session starts",
        None => {}
    }
    out
}
