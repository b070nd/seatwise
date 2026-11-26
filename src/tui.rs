//! `seat tui`: one screen for what is otherwise spread over many commands. A list on the left, a detail card on the
//! right: seats (with usage and their projects), what runs on each seat (start and quit its desktop app, open a
//! terminal on it), projects, presets (with an editor that only takes valid settings), folder groups, agents and the
//! general options. Config changes are checked as they are made and written (keeping comments) only on `w`.

use crate::config::{Apply, Config, Enforcement, NONE};
use crate::draft::{Choice, Draft};
use crate::edit::ConfigFile;
use crate::view::{self, Fact, Health, ProjectView, SeatView};
use crate::{catalog, desktop, paths, preset, ps, sentinel};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use toml_edit::Value as T;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Seats,
    Running,
    Projects,
    Presets,
    Groups,
    Agents,
    General,
}

const TABS: [Tab; 7] = [
    Tab::Seats,
    Tab::Running,
    Tab::Projects,
    Tab::Presets,
    Tab::Groups,
    Tab::Agents,
    Tab::General,
];
const INTERVALS: [&str; 5] = ["1h", "6h", "12h", "24h", "7d"];
const ACCENT: Color = Color::Cyan;

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Seats => "Seats",
            Tab::Running => "Running",
            Tab::Projects => "Projects",
            Tab::Presets => "Presets",
            Tab::Groups => "Folder groups",
            Tab::Agents => "Agents",
            Tab::General => "General",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Tab::Seats => "space on/off  e enforcement  u usage",
            Tab::Running => "d desktop  t terminal  x quit desktop  R restart desktop",
            Tab::Projects => "t terminal here",
            Tab::Presets => "space on/off  e edit settings",
            Tab::Groups => "space on/off  p preset  a apply  s sources",
            Tab::Agents => "space on/off",
            Tab::General => "space/enter change  r run sentinel now",
        }
    }
}

/// The preset editor: a draft, a search over what it can take, and an optional value being typed.
pub struct Editor {
    pub draft: Draft,
    pub query: String,
    pub sel: usize,
    pub experimental: bool,
    /// A value being typed for the selected setting.
    pub input: Option<String>,
    pub note: String,
}

/// What can be started or stopped from the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Act {
    Desktop,
    Quit,
    Restart,
    Term(PathBuf),
}

pub struct App {
    pub file: ConfigFile,
    pub cfg: Config,
    pub tab: usize,
    pub sel: [usize; 7],
    pub dirty: bool,
    pub msg: String,
    pub quit: bool,
    pub report: Option<String>,
    pub editor: Option<Editor>,
    exe: PathBuf,
    seats: Vec<SeatView>,
    projects: Vec<ProjectView>,
    /// Usage lines per seat, loaded on `u` (it asks the vendor's endpoint).
    usage: HashMap<String, std::result::Result<crate::usage::Usage, String>>,
    /// What runs now: the desktop windows and the CLI sessions (the Running tab keeps them fresh).
    desktops: Vec<desktop::Instance>,
    sessions: Vec<ps::Session>,
    /// An action waiting for its key to be pressed again (quitting a window stops its Code sessions).
    pending: Option<(char, String, Act)>,
    /// When set, actions are written here instead of carried out, and nothing is read from the process list (tests).
    pub recorded: Option<Vec<String>>,
}

fn flag(on: bool) -> &'static str {
    if on { "■" } else { "□" }
}

fn color(h: Health) -> Color {
    match h {
        Health::Good => Color::Green,
        Health::Notice => Color::Yellow,
        Health::Bad => Color::Red,
        Health::Off => Color::DarkGray,
    }
}

fn dot(h: Health) -> Span<'static> {
    Span::styled("● ", Style::new().fg(color(h)))
}

fn cycle<'a>(all: &[&'a str], now: &str) -> &'a str {
    let i = all
        .iter()
        .position(|x| *x == now)
        .map_or(0, |i| (i + 1) % all.len());
    all[i]
}

fn fact_line(f: &Fact) -> Line<'static> {
    let value = Style::new().fg(match f.health {
        Health::Good => Color::Reset,
        h => color(h),
    });
    Line::from(vec![
        Span::styled(
            format!("  {:<10}", f.label),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(f.value.clone(), value),
    ])
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {title}"),
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    ))
}

fn bar(pct: f64, width: usize) -> Vec<Span<'static>> {
    let filled = ((pct / 100.0 * width as f64).round() as usize).min(width);
    let c = if pct >= 85.0 {
        Color::Red
    } else if pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    };
    vec![
        Span::styled("━".repeat(filled), Style::new().fg(c)),
        Span::styled("━".repeat(width - filled), Style::new().fg(Color::DarkGray)),
    ]
}

fn project_rows(projects: &[ProjectView], owner: bool) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        format!(
            "  {:<30}{}{:<15}{:>8}  {}",
            "path",
            if owner {
                format!("{:<11}", "seat")
            } else {
                String::new()
            },
            "preset",
            "sessions",
            "last used"
        ),
        Style::new().fg(Color::DarkGray),
    ))];
    for p in projects {
        let preset = p.preset.clone();
        out.push(Line::from(vec![
            dot(p.health),
            Span::raw(format!("{:<30}", view::short_path(&p.path, 28))),
            Span::raw(if owner {
                format!("{:<11}", p.owner)
            } else {
                String::new()
            }),
            Span::raw(format!(
                "{:<15}{:>8}  ",
                view::short_path(&preset, 14),
                p.sessions
            )),
            Span::styled(
                p.last_used.clone().unwrap_or_else(|| "-".into()),
                Style::new().fg(Color::DarkGray),
            ),
        ]));
    }
    out
}

impl App {
    pub fn new(file: ConfigFile, exe: &Path) -> Result<Self> {
        let cfg = file.config()?;
        let mut app = Self {
            file,
            cfg,
            tab: 0,
            sel: [0; 7],
            dirty: false,
            msg: String::new(),
            quit: false,
            report: sentinel::last_report(),
            editor: None,
            exe: exe.to_path_buf(),
            seats: Vec::new(),
            projects: Vec::new(),
            usage: HashMap::new(),
            desktops: Vec::new(),
            sessions: Vec::new(),
            pending: None,
            recorded: None,
        };
        app.refresh();
        Ok(app)
    }

    /// Rebuild the seat and project views after a change.
    fn refresh(&mut self) {
        self.seats = view::seats(&self.cfg, &self.exe);
        self.projects = view::projects(&self.cfg);
    }

    /// Read what runs now. Only the Running tab shows it, so only it pays for reading the process list.
    pub fn tick(&mut self) {
        if self.current() == Tab::Running && self.recorded.is_none() {
            self.desktops = desktop::running(&self.cfg);
            self.sessions = ps::sessions(&self.cfg).unwrap_or_default();
        }
    }

    pub fn current(&self) -> Tab {
        TABS[self.tab]
    }

    fn count(&self) -> usize {
        match self.current() {
            Tab::Seats | Tab::Running => self.cfg.seats.len(),
            Tab::Projects => self.projects.len(),
            Tab::Presets => preset::all(&self.cfg).len(),
            Tab::Groups => self.cfg.groups.len(),
            Tab::Agents => 2,
            Tab::General => 5,
        }
    }

    /// The rows of the current tab, as plain text (also what the tests read).
    pub fn rows(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn items(&self) -> Vec<Line<'static>> {
        let c = &self.cfg;
        match self.current() {
            Tab::Seats => self
                .seats
                .iter()
                .map(|s| {
                    let name = Span::styled(
                        format!("{:<12}", s.name),
                        if s.enabled {
                            Style::new().add_modifier(Modifier::BOLD)
                        } else {
                            Style::new().fg(Color::DarkGray)
                        },
                    );
                    Line::from(vec![
                        dot(s.health),
                        name,
                        Span::styled(format!("{:<7}", s.agent), Style::new().fg(Color::DarkGray)),
                        Span::styled(s.status.clone(), Style::new().fg(color(s.health))),
                    ])
                })
                .collect(),
            Tab::Running => c
                .seats
                .iter()
                .map(|s| {
                    let (word, health) = self.desktop_word(s);
                    let n = self.sessions_of(&s.name).count();
                    Line::from(vec![
                        dot(health),
                        Span::styled(
                            format!("{:<10}", s.name),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("{word:<13}"), Style::new().fg(color(health))),
                        Span::styled(
                            format!("{n} CLI"),
                            Style::new().fg(if n == 0 {
                                Color::DarkGray
                            } else {
                                Color::Reset
                            }),
                        ),
                    ])
                })
                .collect(),
            Tab::Projects => self
                .projects
                .iter()
                .map(|p| {
                    Line::from(vec![
                        dot(p.health),
                        Span::raw(format!("{:<30}", view::short_path(&p.path, 29))),
                        Span::styled(p.owner.clone(), Style::new().fg(Color::DarkGray)),
                    ])
                })
                .collect(),
            Tab::Presets => preset::all(c)
                .iter()
                .map(|p| {
                    let exp = has_experimental(&preset::settings(c, p));
                    Line::from(vec![
                        Span::raw(format!("{} ", flag(p.enabled))),
                        Span::styled(
                            format!("{:<10}", p.name),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{:<10}", p.profiles.join("+")),
                            Style::new().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            if exp { "exp" } else { "" },
                            Style::new().fg(Color::Magenta),
                        ),
                        Span::styled(
                            if p.builtin { "" } else { " yours" },
                            Style::new().fg(ACCENT),
                        ),
                    ])
                })
                .collect(),
            Tab::Groups => c
                .groups
                .iter()
                .map(|g| {
                    Line::from(vec![
                        Span::raw(format!("{} ", flag(g.enabled))),
                        Span::styled(
                            format!("{:<12}", g.name),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!(
                            "preset {:<8} apply {}",
                            g.preset,
                            g.apply.unwrap_or(c.preset_apply).as_str()
                        )),
                        Span::styled(
                            format!(
                                "  sources {}",
                                g.sources.as_ref().map_or("all".into(), |s| s.join(","))
                            ),
                            Style::new().fg(Color::DarkGray),
                        ),
                    ])
                })
                .collect(),
            Tab::Agents => [("claude", "Claude Code"), ("codex", "Codex (ChatGPT)")]
                .iter()
                .map(|(k, label)| {
                    let on = if *k == "claude" {
                        c.agents.claude
                    } else {
                        c.agents.codex
                    };
                    let n = c.seats.iter().filter(|s| s.agent.as_str() == *k).count();
                    Line::from(format!("{} {label:<16} {n} seat(s)", flag(on)))
                })
                .collect(),
            Tab::General => vec![
                Line::from(format!(
                    "{} sentinel checks on start",
                    flag(c.sentinel.enabled)
                )),
                Line::from(format!("  sentinel interval: {}", c.sentinel.interval)),
                Line::from(format!(
                    "{} sentinel desktop notifications",
                    flag(c.sentinel.notify)
                )),
                Line::from(format!(
                    "  presets apply by default: {}",
                    c.preset_apply.as_str()
                )),
                Line::from(format!(
                    "  enforcement (global): {}",
                    c.enforcement.as_str()
                )),
            ],
        }
    }

    /// The detail pane for the selected row, as plain text (what the tests read).
    pub fn detail(&self) -> String {
        self.detail_lines()
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    fn detail_lines(&self) -> Vec<Line<'static>> {
        let c = &self.cfg;
        let i = self.sel[self.tab];
        match self.current() {
            Tab::Seats => self
                .seats
                .get(i)
                .map_or_else(Vec::new, |s| self.seat_card(s)),
            Tab::Running => c
                .seats
                .get(i)
                .map_or_else(Vec::new, |s| self.running_card(s)),
            Tab::Projects => self.projects.get(i).map_or_else(
                || {
                    vec![Line::from(
                        "No project folders yet: `seat link <seat> <folder>`.",
                    )]
                },
                |p| self.project_card(p),
            ),
            Tab::Presets => preset::all(c).get(i).map_or_else(Vec::new, |p| {
                let d =
                    Draft::from_preset(c, &p.name, &p.name).unwrap_or_else(|_| Draft::new(&p.name));
                let mut out = vec![Line::from(Span::styled(
                    p.description.clone(),
                    Style::new().add_modifier(Modifier::ITALIC),
                ))];
                out.push(Line::default());
                out.push(section("own settings"));
                out.extend(
                    d.summary()
                        .lines()
                        .skip(1)
                        .map(|l| Line::from(l.to_string())),
                );
                out.push(Line::default());
                out.push(section("what a session gets"));
                let merged = serde_json::to_string_pretty(&d.preview(c)).unwrap_or_default();
                out.extend(merged.lines().map(|l| Line::from(format!("  {l}"))));
                let notes = d.problems(c);
                if !notes.is_empty() {
                    out.push(Line::default());
                    out.push(section("settings catalog"));
                    out.extend(notes.iter().map(|n| {
                        let col = match n.level {
                            catalog::Level::Error => Color::Red,
                            catalog::Level::Warn => Color::Yellow,
                            catalog::Level::Info => Color::Magenta,
                        };
                        Line::from(Span::styled(
                            format!("  {}", n.message),
                            Style::new().fg(col),
                        ))
                    }));
                }
                out.push(Line::default());
                out.push(Line::from(Span::styled(
                    "  e edits these settings",
                    Style::new().fg(Color::DarkGray),
                )));
                out
            }),
            Tab::Groups => c.groups.get(i).map_or_else(
                || vec![Line::from("No groups yet: `seat group add <folder>`.")],
                |g| {
                    let dir = paths::expand(&g.paths[0]);
                    let seat = match crate::folders::owner_of(
                        c,
                        &crate::folders::normalize(&dir, &paths::home()),
                    ) {
                        Some(crate::config::Owner::Seat(s)) => c.seat(s),
                        _ => c.default_seat.as_deref().and_then(|d| c.seat(d)),
                    };
                    let text = match seat {
                        Some(s) => preset::explain(c, s, &dir, &[]),
                        None => format!(
                            "{} belongs to no seat; `seat explain --seat <seat>`",
                            g.paths[0]
                        ),
                    };
                    text.lines().map(|l| Line::from(l.to_string())).collect()
                },
            ),
            Tab::Agents => vec![Line::from(
                "A turned-off agent's seats get no shell command and can't start.",
            )],
            Tab::General => self
                .report
                .clone()
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "No sentinel findings recorded. Press r to run it.".into())
                .lines()
                .map(|l| Line::from(l.to_string()))
                .collect(),
        }
    }

    fn seat_card(&self, s: &SeatView) -> Vec<Line<'static>> {
        let mut head = vec![
            Span::styled(
                format!(" {} ", s.status.to_uppercase()),
                Style::new()
                    .fg(Color::Black)
                    .bg(color(s.health))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
        ];
        if !s.aliases.is_empty() {
            head.push(Span::styled(
                format!("also {}", s.aliases.join(", ")),
                Style::new().fg(Color::DarkGray),
            ));
        }
        let mut out: Vec<Line> = vec![Line::from(head), Line::default()];
        out.extend(s.facts.iter().map(fact_line));
        out.push(Line::default());
        out.push(section("usage"));
        match self.usage.get(&s.name) {
            None => out.push(Line::from(Span::styled(
                "  u loads this seat's limits (asks the vendor's usage endpoint)",
                Style::new().fg(Color::DarkGray),
            ))),
            Some(Err(e)) => out.push(Line::from(Span::styled(
                format!("  {e}"),
                Style::new().fg(Color::Yellow),
            ))),
            Some(Ok(u)) if u.windows.is_empty() => out.push(Line::from("  no limits reported")),
            Some(Ok(u)) => {
                for w in &u.windows {
                    let mut spans = vec![Span::raw(format!(
                        "  {:<9}",
                        crate::usage::window_label(&w.name)
                    ))];
                    spans.extend(bar(w.utilization, 24));
                    spans.push(Span::raw(format!(" {:>4.0}%", w.utilization)));
                    if let Some(r) = &w.resets_at {
                        spans.push(Span::styled(
                            format!("  resets {}", crate::usage::short_time(r)),
                            Style::new().fg(Color::DarkGray),
                        ));
                    }
                    out.push(Line::from(spans));
                }
            }
        }
        out.push(Line::default());
        out.push(section(&format!("projects ({})", s.projects.len())));
        if s.projects.is_empty() {
            out.push(Line::from(Span::styled(
                format!("  none yet: `seat link {} <folder>`", s.name),
                Style::new().fg(Color::DarkGray),
            )));
        } else {
            out.extend(project_rows(&s.projects, false));
        }
        out
    }

    fn desktop_of(&self, seat: &crate::config::Seat) -> Option<&desktop::Instance> {
        desktop::instance_of(seat, &self.desktops)
    }

    fn sessions_of<'a>(&'a self, seat: &'a str) -> impl Iterator<Item = &'a ps::Session> {
        self.sessions.iter().filter(move |x| x.seat == seat)
    }

    /// The desktop column of the Running tab.
    fn desktop_word(&self, s: &crate::config::Seat) -> (&'static str, Health) {
        if s.agent != crate::config::Agent::Claude {
            return ("no desktop", Health::Off);
        }
        match self.desktop_of(s) {
            None => ("desktop off", Health::Off),
            Some(i) if desktop::problem(&self.cfg, s, i, &self.exe).is_some() => {
                ("desktop: fix", Health::Notice)
            }
            Some(_) => ("desktop on", Health::Good),
        }
    }

    /// Where `t` opens a terminal for a seat: its most recently used project, else the home folder.
    fn term_dir(&self, seat: &str) -> PathBuf {
        self.projects
            .iter()
            .find(|p| p.owner == seat && p.exists)
            .map_or_else(paths::home, |p| paths::expand(&p.path))
    }

    fn running_card(&self, s: &crate::config::Seat) -> Vec<Line<'static>> {
        let dim = Style::new().fg(Color::DarkGray);
        let mut out = vec![section("desktop app")];
        let claude = s.agent == crate::config::Agent::Claude;
        if !claude {
            out.push(Line::from("  Codex has no desktop app here"));
        } else if !cfg!(target_os = "macos") && self.recorded.is_none() {
            out.push(Line::from("  the desktop app is for macOS"));
        } else {
            match self.desktop_of(s) {
                None => out.push(Line::from(Span::styled(
                    "  not running: d starts it on the seat",
                    dim,
                ))),
                Some(i) => {
                    out.push(Line::from(format!("  running: {}", i.label())));
                    if let Some(p) = desktop::problem(&self.cfg, s, i, &self.exe) {
                        out.push(Line::from(Span::styled(
                            format!("  {p}: R restarts it on the seat"),
                            Style::new().fg(Color::Yellow),
                        )));
                    }
                }
            }
        }
        out.push(Line::default());
        let mine: Vec<&ps::Session> = self.sessions_of(&s.name).collect();
        out.push(section(&format!("CLI sessions ({})", mine.len())));
        if mine.is_empty() {
            out.push(Line::from(Span::styled("  none running", dim)));
        }
        for x in mine {
            out.push(Line::from(vec![
                Span::raw(format!("  pid {:<7} {:>4}m  ", x.pid, x.minutes)),
                Span::raw(x.cwd.as_deref().map_or_else(|| "?".into(), paths::contract)),
                Span::styled(
                    x.preset
                        .as_deref()
                        .map_or(String::new(), |p| format!("  {p}")),
                    dim,
                ),
            ]));
        }
        out.push(Line::default());
        out.push(section("keys"));
        let mut keys = Vec::new();
        if claude {
            keys.push("  d  start the desktop app on the seat (never a second window)".to_string());
        }
        keys.push(format!(
            "  t  open a terminal on the seat in {}",
            paths::contract(&self.term_dir(&s.name))
        ));
        if claude {
            keys.push("  x  quit the desktop app, as Cmd-Q does (asks first)".into());
            keys.push("  R  restart the desktop app on the seat (asks first)".into());
        }
        out.extend(keys.into_iter().map(|k| Line::from(Span::styled(k, dim))));
        out
    }

    /// Start or stop something; asks first for what stops a running window.
    fn run_act(&mut self, seat: String, act: Act) {
        let what = match &act {
            Act::Desktop => format!("start seat '{seat}' desktop"),
            Act::Quit => format!("quit seat '{seat}' desktop"),
            Act::Restart => format!("restart seat '{seat}' desktop"),
            Act::Term(dir) => format!(
                "open a terminal on seat '{seat}' in {}",
                paths::contract(dir)
            ),
        };
        if let Some(log) = self.recorded.as_mut() {
            log.push(what.clone());
            self.msg = what;
            return;
        }
        let Some(s) = self.cfg.seat(&seat).cloned() else {
            return;
        };
        let result = match act {
            Act::Desktop => desktop::open(&self.cfg, &s, &self.exe),
            Act::Quit => desktop::quit(&self.cfg, &s),
            Act::Restart => desktop::restart(&self.cfg, &s, &self.exe),
            Act::Term(dir) => {
                let words = crate::term::run_words(
                    &self.exe,
                    &s.name,
                    &self
                        .cfg
                        .terminal
                        .profile
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                    false,
                    &[],
                );
                crate::term::open(&self.cfg, &dir, &crate::term::shell_line(&dir, &words))
                    .map(|()| vec![format!("{what}: done")])
            }
        };
        self.msg = match result {
            Ok(lines) => lines.join("; "),
            Err(e) => crate::error::render(&e)
                .0
                .iter()
                .map(|l| l.trim().trim_start_matches("seat: "))
                .collect::<Vec<_>>()
                .join("; "),
        };
        self.tick();
    }

    /// The Running tab's keys.
    fn running_key(&mut self, k: KeyCode, i: usize) {
        let s = self.cfg.seats[i].clone();
        let claude = s.agent == crate::config::Agent::Claude;
        let running = self.desktop_of(&s).map(|d| d.pid);
        match k {
            KeyCode::Char('t') => {
                let dir = self.term_dir(&s.name);
                self.run_act(s.name, Act::Term(dir));
            }
            KeyCode::Char('d' | 'x' | 'R') if !claude => {
                self.msg = format!(
                    "seat '{}' runs Codex, which has no desktop app here",
                    s.name
                );
            }
            KeyCode::Char('d') => self.run_act(s.name, Act::Desktop),
            KeyCode::Char('x') if running.is_none() => {
                self.msg = format!("seat '{}' desktop isn't running", s.name);
            }
            KeyCode::Char('R') if running.is_none() => self.run_act(s.name, Act::Desktop),
            KeyCode::Char(c @ ('x' | 'R')) => {
                let (act, verb) = if c == 'x' {
                    (Act::Quit, "quit")
                } else {
                    (Act::Restart, "restart")
                };
                self.msg = format!(
                    "{verb} seat '{}' desktop (pid {})? Its Code sessions stop, as with Cmd-Q. {c} again does it, \
                     any other key doesn't",
                    s.name,
                    running.unwrap_or_default()
                );
                self.pending = Some((c, s.name, act));
            }
            _ => {}
        }
    }

    fn project_card(&self, p: &ProjectView) -> Vec<Line<'static>> {
        let preset = if p.preset == "-" {
            "-".to_string()
        } else {
            format!("{} (apply: {})", p.preset, p.apply)
        };
        let mut facts = vec![
            Fact {
                label: "seat",
                value: p.owner.clone(),
                health: Health::Good,
            },
            Fact {
                label: "group",
                value: p.group.clone().unwrap_or_else(|| "-".into()),
                health: Health::Good,
            },
            Fact {
                label: "preset",
                value: preset,
                health: if p.preset_problems.is_empty() {
                    Health::Good
                } else {
                    Health::Bad
                },
            },
            Fact {
                label: "sessions",
                value: p.sessions.to_string(),
                health: Health::Good,
            },
            Fact {
                label: "last used",
                value: p.last_used.clone().unwrap_or_else(|| "-".into()),
                health: Health::Good,
            },
        ];
        if !p.exists {
            facts.push(Fact {
                label: "folder",
                value: "doesn't exist (`seat unlink --missing`)".into(),
                health: Health::Notice,
            });
        }
        let mut out: Vec<Line> = vec![Line::from(Span::styled(
            p.path.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ))];
        out.extend(facts.iter().map(fact_line));
        for m in &p.preset_problems {
            out.push(Line::from(Span::styled(
                format!("  {m}"),
                Style::new().fg(Color::Red),
            )));
        }
        if let Some(seat) = self.cfg.seat(&p.owner) {
            out.push(Line::default());
            out.push(section("what a session here runs with"));
            let text = preset::explain(&self.cfg, seat, &paths::expand(&p.path), &[]);
            out.extend(text.lines().skip(2).map(|l| Line::from(format!("  {l}"))));
        }
        out
    }

    /// Make a change on a copy; keep it only if the config is still valid.
    fn change(&mut self, f: impl FnOnce(&mut ConfigFile, &Config) -> Result<()>) {
        let mut next = self.file.clone();
        match f(&mut next, &self.cfg).and_then(|()| next.config()) {
            Ok(cfg) => {
                self.file = next;
                self.cfg = cfg;
                self.dirty = true;
                self.msg = "changed (w writes it)".into();
                self.refresh();
            }
            Err(e) => self.msg = format!("not changed: {e:#}"),
        }
    }

    pub fn key(&mut self, k: KeyCode) {
        if self.editor.is_some() {
            self.editor_key(k);
            return;
        }
        if let Some((c, seat, act)) = self.pending.take() {
            if k == KeyCode::Char(c) {
                self.run_act(seat, act);
                return;
            }
            self.msg = "not done".into();
        }
        let n = self.count();
        let i = self.sel[self.tab];
        let tab = self.tab;
        self.key_inner(k, n, i);
        if self.tab != tab {
            self.tick();
        }
    }

    fn key_inner(&mut self, k: KeyCode, n: usize, i: usize) {
        match k {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.dirty && !self.msg.starts_with("unsaved") {
                    self.msg = "unsaved changes: w writes them, q again quits without".into();
                } else {
                    self.quit = true;
                }
            }
            KeyCode::Tab | KeyCode::Right => self.tab = (self.tab + 1) % TABS.len(),
            KeyCode::BackTab | KeyCode::Left => self.tab = (self.tab + TABS.len() - 1) % TABS.len(),
            KeyCode::Down | KeyCode::Char('j') if n > 0 => self.sel[self.tab] = (i + 1).min(n - 1),
            KeyCode::Up | KeyCode::Char('k') => self.sel[self.tab] = i.saturating_sub(1),
            KeyCode::Char('w') => match self.file.save() {
                Ok(()) => {
                    self.dirty = false;
                    self.msg = format!("written to {}", paths::config_file().display());
                }
                Err(e) => self.msg = format!("not written: {e:#}"),
            },
            KeyCode::Char('r') if self.current() == Tab::General => {
                let found = sentinel::run(&self.cfg, &[]);
                self.msg = format!("sentinel: {} suggestion(s)", found.len());
                self.report = sentinel::last_report();
            }
            KeyCode::Char('d' | 't' | 'x' | 'R') if self.current() == Tab::Running && n > 0 => {
                self.running_key(k, i);
            }
            KeyCode::Char('t') if self.current() == Tab::Projects && n > 0 => {
                let p = &self.projects[i];
                match self.cfg.seat(&p.owner) {
                    Some(s) => {
                        let (seat, dir) = (s.name.clone(), paths::expand(&p.path));
                        self.run_act(seat, Act::Term(dir));
                    }
                    None => {
                        self.msg = format!("no seat owns {}; open it from the Running tab", p.path)
                    }
                }
            }
            KeyCode::Char('u') if self.current() == Tab::Seats && n > 0 => {
                let seat = self.cfg.seats[i].clone();
                let got = crate::usage::fetch(&seat).map_err(|e| format!("{e:#}"));
                self.msg = match &got {
                    Ok(_) => format!("usage of {} loaded", seat.name),
                    Err(_) => format!("usage of {} not available", seat.name),
                };
                self.usage.insert(seat.name, got);
            }
            KeyCode::Char('e') if self.current() == Tab::Presets && n > 0 => {
                let p = preset::all(&self.cfg)[i].clone();
                match Draft::from_preset(&self.cfg, &p.name, &p.name) {
                    Ok(draft) => {
                        self.editor = Some(Editor {
                            draft,
                            query: String::new(),
                            sel: 0,
                            experimental: false,
                            input: None,
                            note: String::new(),
                        });
                        self.msg = "editing: type to search, ↑/↓ pick, enter set, del remove, tab experimental, esc done".into();
                    }
                    Err(e) => self.msg = format!("{e:#}"),
                }
            }
            _ if n == 0 => {}
            k => self.act(k, i),
        }
    }

    /// The settings the editor offers now.
    pub fn editor_options(&self) -> Vec<Choice> {
        self.editor
            .as_ref()
            .map(|e| e.draft.options(&self.cfg, &e.query, e.experimental))
            .unwrap_or_default()
    }

    fn editor_key(&mut self, k: KeyCode) {
        let options = self.editor_options();
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        // Typing a value.
        if let Some(text) = ed.input.as_mut() {
            match k {
                KeyCode::Char(c) => text.push(c),
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Esc => ed.input = None,
                KeyCode::Enter => {
                    let raw = text.clone();
                    ed.input = None;
                    if let Some(o) = options.get(ed.sel) {
                        let name = o.entry.name.clone();
                        self.editor_set(&name, &raw);
                    }
                }
                _ => {}
            }
            return;
        }
        match k {
            KeyCode::Esc => {
                self.editor = None;
                self.msg = "editor closed (w writes the config)".into();
            }
            KeyCode::Char(c) => {
                ed.query.push(c);
                ed.sel = 0;
            }
            KeyCode::Backspace => {
                ed.query.pop();
                ed.sel = 0;
            }
            KeyCode::Tab => {
                ed.experimental = !ed.experimental;
                ed.sel = 0;
            }
            KeyCode::Down if !options.is_empty() => ed.sel = (ed.sel + 1).min(options.len() - 1),
            KeyCode::Up => ed.sel = ed.sel.saturating_sub(1),
            KeyCode::Delete => {
                if let Some(o) = options.get(ed.sel) {
                    let name = o.entry.name.clone();
                    let mut d = ed.draft.clone();
                    if d.unset(&name) {
                        self.editor_save(d, format!("{name} removed"));
                    }
                }
            }
            KeyCode::Enter => {
                let Some(o) = options.get(ed.sel) else { return };
                if let Some(b) = &o.blocked {
                    ed.note = format!("not now: {b}");
                    return;
                }
                if o.values.is_empty() {
                    ed.input = Some(
                        o.current
                            .as_ref()
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default(),
                    );
                } else {
                    // Cycle to the next value.
                    let now = o.current.as_ref().map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    });
                    let next = match now.and_then(|n| o.values.iter().position(|v| *v == n)) {
                        Some(i) => o.values[(i + 1) % o.values.len()].clone(),
                        None => o.values[0].clone(),
                    };
                    let name = o.entry.name.clone();
                    self.editor_set(&name, &next);
                }
            }
            _ => {}
        }
    }

    fn editor_set(&mut self, key: &str, raw: &str) {
        let cfg = self.cfg.clone();
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        let mut d = ed.draft.clone();
        match d.set(&cfg, key, raw) {
            Ok(notes) => {
                let mut note = format!("{key} = {raw}");
                if let Some(n) = notes.first() {
                    note += &format!("  ({})", n.message);
                }
                self.editor_save(d, note);
            }
            Err(e) => ed.note = format!("{e:#}"),
        }
    }

    /// Put an edited draft into the config (not written until `w`).
    fn editor_save(&mut self, d: Draft, note: String) {
        let saved = d.clone();
        self.change(move |f, cfg| saved.save(cfg, f));
        if let Some(ed) = self.editor.as_mut() {
            if self.msg.starts_with("not changed") {
                ed.note = self.msg.clone();
            } else {
                ed.draft = d;
                ed.note = note;
            }
        }
    }

    fn act(&mut self, k: KeyCode, i: usize) {
        let toggle = matches!(k, KeyCode::Char(' ') | KeyCode::Enter);
        match self.current() {
            Tab::Seats if toggle => {
                let (name, on) = (self.cfg.seats[i].name.clone(), self.cfg.seats[i].enabled);
                self.change(|f, _| f.set_seat_enabled(&name, !on));
            }
            Tab::Seats if k == KeyCode::Char('e') => {
                let s = &self.cfg.seats[i];
                let now = s.enforcement.map_or("global", Enforcement::as_str);
                let next = cycle(&["global", "off", "warn", "strict"], now);
                let name = s.name.clone();
                self.change(|f, _| {
                    let v = (next != "global").then(|| T::from(next));
                    f.set_seat_value(&name, "enforcement", v)
                });
            }
            Tab::Agents if toggle => {
                let key = if i == 0 { "claude" } else { "codex" };
                let on = self.cfg.agents.on(if i == 0 {
                    crate::config::Agent::Claude
                } else {
                    crate::config::Agent::Codex
                });
                self.change(|f, _| f.set(&["agents"], key, on.then(|| T::from(false))));
            }
            Tab::Presets if toggle => {
                let p = preset::all(&self.cfg)[i].clone();
                self.change(|f, cfg| f.set_preset_enabled(cfg, &p.name, !p.enabled));
            }
            Tab::Groups => {
                let g = self.cfg.groups[i].clone();
                match k {
                    _ if toggle => self.change(|f, _| {
                        f.set_group(&g.name, "enabled", g.enabled.then(|| T::from(false)))
                    }),
                    KeyCode::Char('p') => {
                        let mut names: Vec<String> =
                            preset::all(&self.cfg).into_iter().map(|p| p.name).collect();
                        names.push(NONE.into());
                        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                        let next = cycle(&refs, &g.preset).to_string();
                        self.change(|f, _| f.set_group(&g.name, "preset", Some(T::from(next))));
                    }
                    KeyCode::Char('a') => {
                        let now = g.apply.map_or("default", Apply::as_str);
                        let next = cycle(&["default", "session", "project"], now);
                        self.change(|f, _| {
                            f.set_group(
                                &g.name,
                                "apply",
                                (next != "default").then(|| T::from(next)),
                            )
                        });
                    }
                    KeyCode::Char('s') => {
                        let now = g
                            .sources
                            .as_ref()
                            .map_or("all".to_string(), |s| s.join(","));
                        let next = cycle(&["all", "user,local", "user"], &now);
                        let v = (next != "all")
                            .then(|| T::from(next.split(',').collect::<toml_edit::Array>()));
                        self.change(|f, _| f.set_group(&g.name, "sources", v));
                    }
                    _ => {}
                }
            }
            Tab::General if toggle => {
                let s = self.cfg.sentinel.clone();
                let apply = self.cfg.preset_apply;
                let enf = self.cfg.enforcement;
                self.change(move |f, _| match i {
                    0 => f.set(&["sentinel"], "enabled", Some(T::from(!s.enabled))),
                    1 => f.set(
                        &["sentinel"],
                        "interval",
                        Some(T::from(cycle(&INTERVALS, &s.interval))),
                    ),
                    2 => f.set(&["sentinel"], "notify", Some(T::from(!s.notify))),
                    3 => {
                        let next = if apply == Apply::Session {
                            "project"
                        } else {
                            "session"
                        };
                        f.set(&[], "preset_apply", Some(T::from(next)))
                    }
                    _ => f.set(
                        &[],
                        "enforcement",
                        Some(T::from(cycle(&["off", "warn", "strict"], enf.as_str()))),
                    ),
                });
            }
            _ => {}
        }
    }

    fn detail_title(&self) -> (String, Color) {
        let i = self.sel[self.tab];
        match self.current() {
            Tab::Seats => self.seats.get(i).map_or((" seat ".into(), ACCENT), |s| {
                (format!(" {} · {} ", s.name, s.agent), color(s.health))
            }),
            Tab::Running => (
                self.cfg
                    .seats
                    .get(i)
                    .map_or(" running ".into(), |s| format!(" {} · running ", s.name)),
                ACCENT,
            ),
            Tab::Projects => (
                " project ".into(),
                self.projects.get(i).map_or(ACCENT, |p| color(p.health)),
            ),
            Tab::Presets => (
                preset::all(&self.cfg)
                    .get(i)
                    .map_or(" preset ".into(), |p| format!(" preset {} ", p.name)),
                ACCENT,
            ),
            Tab::Groups => (" what a session there runs with ".into(), ACCENT),
            _ => (" details ".into(), ACCENT),
        }
    }

    fn draw_editor(&self, frame: &mut ratatui::Frame, area: Rect, ed: &Editor) {
        let options = self.editor_options();
        let [head, list, foot] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(6),
        ])
        .areas(area);
        let search = Line::from(vec![
            Span::styled(" search ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                format!("{}▏", ed.query),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if ed.experimental {
                    "   experimental only (tab)"
                } else {
                    "   all (tab: experimental)"
                },
                Style::new().fg(Color::Magenta),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(vec![
                search,
                Line::from(Span::styled(
                    format!(
                        " applied in {}; only what Claude Code honors there is offered",
                        ed.draft.target.label()
                    ),
                    Style::new().fg(Color::DarkGray),
                )),
            ])
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(Color::Magenta))
                    .title(format!(" editing preset {} ", ed.draft.name)),
            ),
            head,
        );
        let items: Vec<ListItem> = options
            .iter()
            .map(|o| {
                let mut spans = vec![Span::styled(
                    format!("{:<44}", o.entry.name),
                    if o.blocked.is_some() {
                        Style::new().fg(Color::DarkGray)
                    } else {
                        Style::new()
                    },
                )];
                if let Some(v) = &o.current {
                    spans.push(Span::styled(
                        format!(" = {v}"),
                        Style::new().fg(Color::Green),
                    ));
                }
                if o.entry.experimental() {
                    spans.push(Span::styled("  exp", Style::new().fg(Color::Magenta)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let mut state = ListState::default().with_selected((!options.is_empty()).then_some(ed.sel));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(
                    Style::new()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(format!(" {} setting(s) ", options.len())),
                ),
            list,
            &mut state,
        );
        let mut info = Vec::new();
        if let Some(o) = options.get(ed.sel) {
            info.push(Line::from(Span::styled(
                o.entry.meaning.clone(),
                Style::new().add_modifier(Modifier::ITALIC),
            )));
            let values = if o.values.is_empty() {
                o.entry.type_label()
            } else {
                o.values.join(" · ")
            };
            info.push(Line::from(vec![
                Span::styled("values ", Style::new().fg(Color::DarkGray)),
                Span::raw(values),
            ]));
            if let Some(b) = &o.blocked {
                info.push(Line::from(Span::styled(
                    format!("not now: {b}"),
                    Style::new().fg(Color::Yellow),
                )));
            }
        }
        if let Some(text) = &ed.input {
            info.push(Line::from(vec![
                Span::styled("value: ", Style::new().fg(Color::Magenta)),
                Span::styled(
                    format!("{text}▏"),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  enter sets, esc cancels",
                    Style::new().fg(Color::DarkGray),
                ),
            ]));
        } else if !ed.note.is_empty() {
            info.push(Line::from(Span::styled(
                ed.note.clone(),
                Style::new().fg(ACCENT),
            )));
        }
        frame.render_widget(
            Paragraph::new(info)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().border_type(BorderType::Rounded)),
            foot,
        );
    }

    fn draw(&self, frame: &mut ratatui::Frame) {
        let [top, body, foot] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .areas(frame.area());
        let titles: Vec<Line> = TABS
            .iter()
            .map(|t| Line::from(format!(" {} ", t.title())))
            .collect();
        frame.render_widget(
            Tabs::new(titles)
                .select(self.tab)
                .divider("│")
                .highlight_style(
                    Style::new()
                        .fg(Color::Black)
                        .bg(ACCENT)
                        .add_modifier(Modifier::BOLD),
                )
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(Style::new().fg(ACCENT))
                        .title(Line::from(vec![
                            Span::styled(
                                " seatwise ",
                                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                if self.dirty { "● unsaved " } else { "" },
                                Style::new().fg(Color::Yellow),
                            ),
                        ])),
                ),
            top,
        );
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(36), Constraint::Percentage(64)])
                .areas(body);
        let items: Vec<ListItem> = self.items().into_iter().map(ListItem::new).collect();
        let mut state = ListState::default().with_selected(Some(self.sel[self.tab]));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(
                    Style::new()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▌")
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(format!(" {} ", self.current().title())),
                ),
            left,
            &mut state,
        );
        match &self.editor {
            Some(ed) => self.draw_editor(frame, right, ed),
            None => {
                let (title, border) = self.detail_title();
                frame.render_widget(
                    Paragraph::new(self.detail_lines())
                        .wrap(Wrap { trim: false })
                        .block(
                            Block::bordered()
                                .border_type(BorderType::Rounded)
                                .border_style(Style::new().fg(border))
                                .title(title.bold()),
                        ),
                    right,
                );
            }
        }
        let help = if self.editor.is_some() {
            "type search  ↑/↓ pick  enter set  del remove  tab experimental  esc done".to_string()
        } else {
            let h = self.current().help();
            format!(
                "{h}{}←/→ tab  ↑/↓ move  w write  q quit",
                if h.is_empty() { "" } else { "  ·  " }
            )
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(" {help}"),
                    Style::new().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    format!(" {}", self.msg),
                    Style::new().fg(ACCENT),
                )),
            ])
            .wrap(Wrap { trim: false }),
            foot,
        );
    }
}

fn has_experimental(settings: &serde_json::Value) -> bool {
    catalog::check(settings, catalog::Target::Session)
        .iter()
        .any(|p| p.level == catalog::Level::Info && p.message.contains("experimental"))
}

/// Run the screen on the config file.
pub fn run(exe: &Path) -> Result<()> {
    let mut app = App::new(ConfigFile::open()?, exe)?;
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        while !app.quit {
            terminal.draw(|f| app.draw(f))?;
            // Without a key for a while, look again at what runs (a window started or quit elsewhere).
            if !event::poll(std::time::Duration::from_secs(2))? {
                app.tick();
                continue;
            }
            if let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                app.key(k.code);
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

#[doc(hidden)]
pub fn app_at(path: &Path) -> Result<App> {
    App::new(ConfigFile::open_at(path)?, Path::new("/usr/local/bin/seat"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn app(text: &str) -> (tempfile::TempDir, App) {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.toml");
        std::fs::write(&p, text).unwrap();
        let a = app_at(&p).unwrap();
        (t, a)
    }

    const CFG: &str = "# mine\n[[seat]]\nname = \"w\"\nconfig_dir = \"~/.cw\"\n\n[[group]]\nname = \"g\"\npaths = [\"~/code\"]\npreset = \"build\"\n";

    fn goto(a: &mut App, tab: Tab) {
        while a.current() != tab {
            a.key(KeyCode::Right);
        }
    }

    #[test]
    fn toggles_cycle_and_write_keeping_comments() {
        let (t, mut a) = app(CFG);
        a.key(KeyCode::Char(' ')); // seat w off
        assert!(!a.cfg.seats[0].enabled);
        a.key(KeyCode::Char('e')); // enforcement global → off
        assert_eq!(a.cfg.seats[0].enforcement, Some(Enforcement::Off));
        goto(&mut a, Tab::Agents);
        a.key(KeyCode::Down);
        a.key(KeyCode::Char(' ')); // codex off
        assert!(!a.cfg.agents.codex);
        goto(&mut a, Tab::Presets); // explore first
        a.key(KeyCode::Char(' '));
        assert!(!preset::get(&a.cfg, "explore").unwrap().enabled);
        assert_eq!(
            preset::get(&a.cfg, "explore").unwrap().profiles,
            ["light"],
            "a disabled built-in keeps its content"
        );
        goto(&mut a, Tab::Groups);
        a.key(KeyCode::Char('p')); // build → review
        a.key(KeyCode::Char('a')); // default → session
        a.key(KeyCode::Char('s')); // all → user,local
        let g = &a.cfg.groups[0];
        assert_eq!(g.preset, "review");
        assert_eq!(g.apply, Some(Apply::Session));
        assert_eq!(
            g.sources.as_deref(),
            Some(&["user".to_string(), "local".to_string()][..])
        );
        goto(&mut a, Tab::General);
        a.key(KeyCode::Char(' '));
        a.key(KeyCode::Down);
        a.key(KeyCode::Enter); // 24h → 7d
        assert!(a.cfg.sentinel.enabled);
        assert_eq!(a.cfg.sentinel.interval, "7d");
        a.key(KeyCode::Char('q'));
        assert!(!a.quit, "asks first");
        a.key(KeyCode::Char('w'));
        a.key(KeyCode::Char('q'));
        assert!(a.quit);
        let text = std::fs::read_to_string(t.path().join("config.toml")).unwrap();
        assert!(text.starts_with("# mine"), "{text}");
        assert!(
            text.contains("[preset.explore]") && !text.contains("\n[preset]\n"),
            "{text}"
        );
        let back = Config::load_from(&t.path().join("config.toml")).unwrap();
        assert_eq!(back.groups[0].preset, "review");
        assert!(!back.seats[0].enabled);
    }

    #[test]
    fn the_editor_only_takes_valid_settings() {
        let (t, mut a) = app(CFG);
        goto(&mut a, Tab::Presets);
        a.key(KeyCode::Down); // build
        a.key(KeyCode::Char('e'));
        for c in "effortlevel".chars() {
            a.key(KeyCode::Char(c));
        }
        let o = a.editor_options();
        assert_eq!(
            o[0].entry.name,
            "effortLevel",
            "{:?}",
            o.iter().map(|o| &o.entry.name).collect::<Vec<_>>()
        );
        a.key(KeyCode::Enter); // → low
        a.key(KeyCode::Enter); // → medium
        assert_eq!(
            a.editor.as_ref().unwrap().draft.settings["effortLevel"],
            "medium"
        );
        // A typed value Claude Code wouldn't take is refused.
        for _ in 0.."effortlevel".len() {
            a.key(KeyCode::Backspace);
        }
        for c in "max_thinking".chars() {
            a.key(KeyCode::Char(c));
        }
        a.key(KeyCode::Enter);
        for c in "lots".chars() {
            a.key(KeyCode::Char(c));
        }
        a.key(KeyCode::Enter);
        assert!(
            a.editor.as_ref().unwrap().note.contains("not set"),
            "{}",
            a.editor.as_ref().unwrap().note
        );
        a.key(KeyCode::Esc);
        a.key(KeyCode::Char('w'));
        let text = std::fs::read_to_string(t.path().join("config.toml")).unwrap();
        assert!(
            text.contains("[preset.build.settings]") && text.contains("effortLevel = \"medium\""),
            "{text}"
        );
    }

    #[test]
    fn the_running_tab_starts_and_asks_before_it_quits() {
        let (_t, mut a) = app(&format!(
            "{CFG}\n[[seat]]\nname = \"x\"\nagent = \"codex\"\nconfig_dir = \"~/.cx\"\n"
        ));
        a.recorded = Some(Vec::new());
        goto(&mut a, Tab::Running);
        assert_eq!(a.rows().len(), 2, "{:?}", a.rows());
        assert!(a.rows()[0].contains("desktop off"), "{:?}", a.rows());
        assert!(a.rows()[1].contains("no desktop"), "{:?}", a.rows());
        assert!(
            a.detail().contains("d starts it on the seat"),
            "{}",
            a.detail()
        );
        a.key(KeyCode::Char('d'));
        a.key(KeyCode::Char('t'));
        a.key(KeyCode::Char('x')); // nothing runs: nothing to quit
        assert!(a.msg.contains("isn't running"), "{}", a.msg);
        a.key(KeyCode::Char('R')); // nothing runs: a restart is a start
        // A running window: x and R ask first; another key cancels.
        a.desktops = vec![desktop::Instance {
            pid: 42,
            started: "now".into(),
            app: desktop::DEFAULT_APP.into(),
            data_dir: Some(paths::home().join("Library/Application Support/Claude-w")),
            seat: Some("w".into()),
            config_dir: Some(paths::home().join(".cw").display().to_string()),
            browser: None,
        }];
        assert!(a.rows()[0].contains("desktop on"), "{:?}", a.rows());
        a.key(KeyCode::Char('x'));
        assert!(
            a.msg.contains("pid 42") && a.msg.contains("x again"),
            "{}",
            a.msg
        );
        a.key(KeyCode::Down); // cancels, and moves
        a.key(KeyCode::Up);
        a.key(KeyCode::Char('R'));
        a.key(KeyCode::Char('R'));
        a.key(KeyCode::Down);
        a.key(KeyCode::Char('d')); // the Codex seat
        assert!(a.msg.contains("Codex"), "{}", a.msg);
        assert_eq!(
            a.recorded.as_deref().unwrap(),
            [
                "start seat 'w' desktop".to_string(),
                "open a terminal on seat 'w' in ~".to_string(),
                "start seat 'w' desktop".into(),
                "restart seat 'w' desktop".into(),
            ]
        );
    }

    #[test]
    #[ignore = "prints the screens: cargo test -- --ignored --nocapture show_screens"]
    fn show_screens() {
        let (_t, mut a) = app(&format!(
            "{CFG}\n[[folder]]\npath = \"~/code/app\"\nowner = \"w\"\n"
        ));
        let mut term = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
        for _ in 0..2 {
            term.draw(|f| a.draw(f)).unwrap();
            let b = term.backend().buffer().clone();
            for y in 0..b.area.height {
                let line: String = (0..b.area.width)
                    .map(|x| b[(x, y)].symbol().to_string())
                    .collect();
                println!("{line}");
            }
            a.key(KeyCode::Right);
        }
    }

    #[test]
    fn every_tab_draws() {
        let (_t, mut a) = app(CFG);
        let mut term = ratatui::Terminal::new(TestBackend::new(120, 40)).unwrap();
        for _ in TABS {
            term.draw(|f| a.draw(f)).unwrap();
            a.key(KeyCode::Right);
        }
        goto(&mut a, Tab::Presets);
        a.key(KeyCode::Char('e'));
        term.draw(|f| a.draw(f)).unwrap();
        let screen = format!("{:?}", term.backend().buffer());
        assert!(screen.contains("editing preset"), "editor drawn");
    }
}
