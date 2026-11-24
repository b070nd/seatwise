//! `seat hybrid`: set up the command-line side of a project next to the desktop app, in one step.
//!
//! From a desktop (or CLI) session it works out the session's seat and project, then makes sure the CLI side is
//! ready: the folder belongs to that seat, the seat has the seatwise skills, the project has the use-case preset
//! you want, and the terminal's session-only settings are ones Claude Code honors there. Then it opens a terminal on
//! the same seat and project.
//!
//! [`plan`] works all of that out without changing anything: each step is ok, a change it will make, or an open
//! choice with the options and the flag that picks each one. `seat hybrid --check --json` hands the plan to the
//! `/seatwise:hybrid` skill, which asks the user only about the open choices. [`carry_out`] makes the changes.

use crate::catalog::{self, Level, Target};
use crate::config::{Agent, Config, Owner, Seat};
use crate::edit::ConfigFile;
use crate::{folders, link, paths, preset, skills, term};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
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
        return cfg.seat_named(s);
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

/// What `seat hybrid` found and will do.
#[derive(Debug, Serialize)]
pub struct Plan {
    pub seat: String,
    pub agent: String,
    pub project: String,
    pub steps: Vec<Step>,
    /// What the user still has to decide; empty when the plan can run as it is.
    pub choices: Vec<Choice>,
    /// Profiles the terminal runs with (session only).
    pub with: Vec<String>,
    /// The command the terminal runs.
    pub terminal: String,
    #[serde(skip)]
    root: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct Step {
    /// owner, skills, preset, session or terminal.
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Already as it should be.
    Ok,
    /// `seat hybrid` (without --check) changes it.
    Change,
    /// Up to the user: see the matching choice.
    Open,
    /// For information.
    Note,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    /// The step it belongs to.
    pub step: &'static str,
    pub question: String,
    pub options: Vec<Opt>,
}

#[derive(Debug, Serialize)]
pub struct Opt {
    pub value: String,
    pub label: String,
    /// What to add to `seat hybrid` for this option; none: leave it as it is.
    pub flag: Option<String>,
    /// Turns on experimental Claude Code features.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub experimental: bool,
}

impl Plan {
    /// The report, one line per step.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("seat     {} ({})", self.seat, self.agent),
            format!("project  {}", self.project),
        ];
        out.extend(
            self.steps
                .iter()
                .map(|s| format!("{:<8} {}", s.name, s.detail)),
        );
        for c in &self.choices {
            let opts: Vec<String> = c
                .options
                .iter()
                .map(|o| match &o.flag {
                    Some(f) => f.clone(),
                    None => o.value.clone(),
                })
                .collect();
            out.push(format!("choose   {}: {}", c.question, opts.join(" | ")));
        }
        out
    }

    fn step(&self, name: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.name == name)
    }
}

fn step(name: &'static str, status: Status, detail: impl Into<String>) -> Step {
    Step {
        name,
        status,
        detail: detail.into(),
    }
}

/// The settings a set of profiles gives a session, merged in order.
fn session_settings(cfg: &Config, with: &[String]) -> Result<Value> {
    let mut merged = json!({});
    for name in with {
        let s = crate::plan::settings_of(cfg, name).with_context(|| {
            format!("unknown profile '{name}' (`seat profile <seat>` lists them)")
        })?;
        crate::plan::merge(&mut merged, &s);
    }
    Ok(merged)
}

/// Whether settings turn on anything the catalog marks experimental.
fn experimental(settings: &Value) -> bool {
    let mut keys: Vec<String> = Vec::new();
    if let Some(obj) = settings.as_object() {
        for (k, v) in obj {
            match (k.as_str(), v.as_object()) {
                ("env", Some(env)) => keys.extend(env.keys().cloned()),
                _ => keys.push(k.clone()),
            }
        }
    }
    keys.iter()
        .any(|k| catalog::lookup(k).is_some_and(|e| e.experimental()))
}

/// Work out what `seat hybrid` would do. Changes nothing. Refuses another seat's folder and a disabled seat.
pub fn plan(cfg: &Config, exe: &Path, opts: &Options) -> Result<Plan> {
    let root = project_dir(opts)?;
    let seat = pick_seat(cfg, opts, &root)?;
    if !cfg.is_enabled(seat) {
        bail!(
            "seat '{}' is disabled (`seat enable {}`)",
            seat.name,
            seat.name
        );
    }
    let n = &seat.name;
    let shown = paths::contract(&root);
    let mut steps = Vec::new();
    let mut choices = Vec::new();

    // 1. The folder belongs to this seat.
    steps.push(match folders::owner_of(cfg, &root) {
        Some(Owner::Seat(s)) if s == seat.name => step("owner", Status::Ok, format!("{n}: ok")),
        Some(Owner::Seat(other)) => bail!(
            "{shown} belongs to seat '{other}', not '{n}'; open the terminal from a '{other}' session"
        ),
        Some(Owner::Nobody) => bail!("{shown} is closed to every seat (owner \"none\")"),
        Some(Owner::Any) => step(
            "owner",
            Status::Note,
            "shared start folder: any seat may start here",
        ),
        None if opts.link => step(
            "owner",
            Status::Change,
            format!("{shown} would belong to {n}"),
        ),
        None => {
            choices.push(Choice {
                step: "owner",
                question: format!("should {shown} belong to seat '{n}'"),
                options: vec![
                    Opt {
                        value: "yes".into(),
                        label: format!("assign it to {n}, so `seat run auto` and the guard know it"),
                        flag: Some("--link".into()),
                        experimental: false,
                    },
                    Opt {
                        value: "no".into(),
                        label: "leave it unassigned".into(),
                        flag: None,
                        experimental: false,
                    },
                ],
            });
            step(
                "owner",
                Status::Open,
                format!(
                    "not assigned: `seat hybrid --link` gives it to {n}, so `seat run auto` and the guard know it"
                ),
            )
        }
    });

    // 2. The seat has the seatwise skills.
    let have = skills::state(seat, exe);
    steps.push(if have.current() {
        step("skills", Status::Ok, "up to date")
    } else {
        step("skills", Status::Change, have.summary())
    });

    // 3. The project's preset.
    if let Some(p) = &opts.preset {
        if p != crate::config::NONE && preset::get(cfg, p).is_none() {
            bail!("{}", preset::unknown(cfg, p));
        }
        steps.push(step("preset", Status::Change, format!("would become {p}")));
    } else {
        let r = preset::resolve(cfg, seat, &root);
        steps.push(match (&r.preset, &r.group) {
            (Some(p), Some(g)) => step(
                "preset",
                Status::Ok,
                format!("{p} from group '{g}', applied per {}", r.apply.as_str()),
            ),
            (None, Some(g)) => step("preset", Status::Ok, format!("none (group '{g}')")),
            _ => {
                let mut options: Vec<Opt> = preset::all(cfg)
                    .into_iter()
                    .filter(|p| p.enabled)
                    .map(|p| Opt {
                        experimental: experimental(&preset::settings(cfg, &p)),
                        flag: Some(format!("--preset {}", p.name)),
                        label: p.description.clone(),
                        value: p.name,
                    })
                    .collect();
                options.push(Opt {
                    value: "none".into(),
                    label: "no preset for this project".into(),
                    flag: None,
                    experimental: false,
                });
                choices.push(Choice {
                    step: "preset",
                    question: "a use-case preset for this project".into(),
                    options,
                });
                step(
                    "preset",
                    Status::Open,
                    "none: `--preset explore|build|review|quick` sets one for this project",
                )
            }
        });
    }

    // 4. Session-only settings for the terminal, checked against the catalog.
    let with: Vec<String> = if opts.with.is_empty() {
        cfg.terminal.profile.iter().cloned().collect()
    } else {
        opts.with.clone()
    };
    if seat.agent == Agent::Claude {
        let settings = session_settings(cfg, &with)?;
        let problems = catalog::check(&settings, Target::Session);
        let errors: Vec<_> = problems
            .iter()
            .filter(|p| p.level == Level::Error)
            .collect();
        if !errors.is_empty() {
            bail!(
                "--with {} gives settings Claude Code would not take:\n{}",
                with.join(","),
                catalog::report(&problems)
            );
        }
        if with.is_empty() {
            let options: Vec<Opt> = crate::plan::all(cfg)
                .into_iter()
                .filter_map(|(name, label)| {
                    let s = crate::plan::settings_of(cfg, &name)?;
                    catalog::errors(&s, Target::Session)
                        .is_empty()
                        .then(|| Opt {
                            experimental: experimental(&s),
                            flag: Some(format!("--with {name}")),
                            value: name,
                            label,
                        })
                })
                .chain(std::iter::once(Opt {
                    value: "none".into(),
                    label: "the seat's own settings".into(),
                    flag: None,
                    experimental: false,
                }))
                .collect();
            choices.push(Choice {
                step: "session",
                question: "session-only settings for the terminal".into(),
                options,
            });
            steps.push(step(
                "session",
                Status::Open,
                "the seat's own settings; `--with <profile>` adds session-only ones (`--with experimental`: Code Mode)",
            ));
        } else {
            let warns = problems.iter().filter(|p| p.level == Level::Warn).count();
            let exp = if experimental(&settings) {
                ", experimental"
            } else {
                ""
            };
            let tail = if warns > 0 {
                format!(" ({warns} warning(s): `seat settings check` explains)")
            } else {
                String::new()
            };
            steps.push(step(
                "session",
                Status::Ok,
                format!("{}{exp}: valid for a session{tail}", with.join(", ")),
            ));
        }
    }

    // 5. The terminal.
    let words = term::run_words(exe, &seat.name, &with, false, &[]);
    let line = term::shell_line(&root, &words);
    let with_note = if with.is_empty() {
        String::new()
    } else {
        format!(", with {}", with.join(", "))
    };
    steps.push(step(
        "terminal",
        Status::Note,
        format!("not opened (`seat hybrid` opens it{with_note})"),
    ));

    Ok(Plan {
        seat: seat.name.clone(),
        agent: seat.agent.as_str().to_owned(),
        project: shown,
        steps,
        choices,
        with,
        terminal: line,
        root,
    })
}

/// Make the plan's changes, then open the terminal (unless `no_open` / `print`). Returns the report.
pub fn carry_out(exe: &Path, opts: &Options, mut plan: Plan) -> Result<Vec<String>> {
    let mut cfg = Config::load()?;
    let n = plan.seat.clone();
    let seat = cfg.seat(&n).context("seat gone")?.clone();
    let root = plan.root.clone();
    let shown = plan.project.clone();
    let mut done: Vec<(&'static str, String)> = Vec::new();

    if plan
        .step("owner")
        .is_some_and(|s| s.status == Status::Change)
    {
        let mut file = ConfigFile::open()?;
        file.link(&root, &n);
        file.save()?;
        cfg = Config::load()?;
        done.push(("owner", format!("{shown} now belongs to {n}")));
    }
    if plan
        .step("skills")
        .is_some_and(|s| s.status == Status::Change)
    {
        let installed = skills::install(&cfg, &seat, exe)?;
        done.push((
            "skills",
            format!(
                "installed in {}{}",
                paths::contract(&installed.dir),
                install_note(&installed)
            ),
        ));
    }
    if let Some(p) = &opts.preset {
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
        done.push(("preset", format!("{p} (group '{name}')")));
    }
    let with_note = if plan.with.is_empty() {
        String::new()
    } else {
        format!(", with {}", plan.with.join(", "))
    };
    if opts.no_open {
        // The plan's line stands.
    } else if opts.print {
        done.push(("terminal", plan.terminal.clone()));
    } else {
        term::open(&cfg, &root, &plan.terminal)?;
        done.push(("terminal", format!("opened on {n}{with_note}")));
    }
    for (name, detail) in done {
        if let Some(s) = plan.steps.iter_mut().find(|s| s.name == name) {
            s.status = Status::Ok;
            s.detail = detail;
        }
    }
    plan.choices.clear();
    Ok(plan.lines())
}

/// `seat hybrid`: the plan, and unless `--check`, carried out. Returns the report, one line per step.
pub fn run(exe: &Path, opts: &Options) -> Result<Vec<String>> {
    let cfg = Config::load()?;
    let plan = plan(&cfg, exe, opts)?;
    if opts.check {
        return Ok(plan.lines());
    }
    carry_out(exe, opts, plan)
}

/// What to add after "installed in <dir>": registration and removed old skills.
pub fn install_note(done: &skills::Installed) -> String {
    let mut out = String::new();
    if !done.retired.is_empty() {
        out += &format!("; removed the old plugin {}", done.retired.join(", "));
    }
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
