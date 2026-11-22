//! `seat tui`: one screen for what is otherwise spread over many commands. Seats, agents, presets, folder groups
//! and the general options (sentinel, enforcement, how presets apply) can be switched on and off and cycled through
//! their values. Changes are checked as they are made and written (keeping comments) only on `w`.

use crate::config::{Apply, Config, Enforcement, NONE};
use crate::edit::ConfigFile;
use crate::{paths, preset, sentinel};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use std::path::Path;
use toml_edit::Value as T;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Seats,
    Agents,
    Presets,
    Groups,
    General,
}

const TABS: [Tab; 5] = [
    Tab::Seats,
    Tab::Agents,
    Tab::Presets,
    Tab::Groups,
    Tab::General,
];
const INTERVALS: [&str; 5] = ["1h", "6h", "12h", "24h", "7d"];

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Seats => "Seats",
            Tab::Agents => "Agents",
            Tab::Presets => "Presets",
            Tab::Groups => "Folder groups",
            Tab::General => "General",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Tab::Seats => "space on/off  e enforcement",
            Tab::Agents => "space on/off",
            Tab::Presets => "space on/off",
            Tab::Groups => "space on/off  p preset  a apply  s sources",
            Tab::General => "space/enter change  r run sentinel now",
        }
    }
}

pub struct App {
    pub file: ConfigFile,
    pub cfg: Config,
    pub tab: usize,
    pub sel: [usize; 5],
    pub dirty: bool,
    pub msg: String,
    pub quit: bool,
    pub report: Option<String>,
}

fn flag(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

fn cycle<'a>(all: &[&'a str], now: &str) -> &'a str {
    let i = all
        .iter()
        .position(|x| *x == now)
        .map_or(0, |i| (i + 1) % all.len());
    all[i]
}

impl App {
    pub fn new(file: ConfigFile) -> Result<Self> {
        let cfg = file.config()?;
        Ok(Self {
            file,
            cfg,
            tab: 0,
            sel: [0; 5],
            dirty: false,
            msg: String::new(),
            quit: false,
            report: sentinel::last_report(),
        })
    }

    pub fn current(&self) -> Tab {
        TABS[self.tab]
    }

    /// The rows of the current tab.
    pub fn rows(&self) -> Vec<String> {
        let c = &self.cfg;
        match self.current() {
            Tab::Seats => c
                .seats
                .iter()
                .map(|s| {
                    let agent_off = if c.agents.on(s.agent) {
                        ""
                    } else {
                        "  (agent off)"
                    };
                    let enf = match s.enforcement {
                        Some(e) => e.as_str().to_string(),
                        None => format!("{} (global)", c.enforcement.as_str()),
                    };
                    format!(
                        "{} {:<14} {:<7} enforcement {enf}{agent_off}",
                        flag(s.enabled),
                        s.name,
                        s.agent.as_str()
                    )
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
                    format!("{} {label:<16} {n} seat(s)", flag(on))
                })
                .collect(),
            Tab::Presets => preset::all(c)
                .iter()
                .map(|p| {
                    format!(
                        "{} {:<10} {:<12} {}",
                        flag(p.enabled),
                        p.name,
                        p.profiles.join("+"),
                        p.description
                    )
                })
                .collect(),
            Tab::Groups => c
                .groups
                .iter()
                .map(|g| {
                    format!(
                        "{} {:<12} preset {:<8} apply {:<8} sources {:<14} {}",
                        flag(g.enabled),
                        g.name,
                        g.preset,
                        g.apply.unwrap_or(c.preset_apply).as_str(),
                        g.sources.as_ref().map_or("all".into(), |s| s.join(",")),
                        g.paths.join(", ")
                    )
                })
                .collect(),
            Tab::General => vec![
                format!("{} sentinel checks on start", flag(c.sentinel.enabled)),
                format!("    sentinel interval: {}", c.sentinel.interval),
                format!("{} sentinel desktop notifications", flag(c.sentinel.notify)),
                format!("    presets apply by default: {}", c.preset_apply.as_str()),
                format!("    enforcement (global): {}", c.enforcement.as_str()),
            ],
        }
    }

    /// The detail pane for the selected row.
    pub fn detail(&self) -> String {
        let c = &self.cfg;
        let i = self.sel[self.tab];
        match self.current() {
            Tab::Seats => c.seats.get(i).map_or_else(String::new, |s| {
                format!(
                    "config  {}\nfolders {}\nprofile {}\nlink    {}\n",
                    paths::contract(&s.config_dir()),
                    c.folders_of(&s.name)
                        .iter()
                        .map(|f| paths::contract(f))
                        .collect::<Vec<_>>()
                        .join(", "),
                    s.profile.as_deref().unwrap_or("-"),
                    match s.link() {
                        crate::config::Link::System => "system".into(),
                        crate::config::Link::Copy => "copy".into(),
                        crate::config::Link::Command(a) => a.join(" "),
                    }
                )
            }),
            Tab::Agents => {
                "A turned-off agent's seats get no shell command and can't start.".into()
            }
            Tab::Presets => preset::all(c).get(i).map_or_else(String::new, |p| {
                serde_json::to_string_pretty(&preset::settings(c, p)).unwrap_or_default()
            }),
            Tab::Groups => c.groups.get(i).map_or_else(
                || "No groups yet: `seat group add <folder>`.".into(),
                |g| {
                    let dir = paths::expand(&g.paths[0]);
                    let seat = match crate::folders::owner_of(
                        c,
                        &crate::folders::normalize(&dir, &paths::home()),
                    ) {
                        Some(crate::config::Owner::Seat(s)) => c.seat(s),
                        _ => c.default_seat.as_deref().and_then(|d| c.seat(d)),
                    };
                    match seat {
                        Some(s) => preset::explain(c, s, &dir, &[]),
                        None => format!(
                            "{} belongs to no seat; `seat explain --seat <seat>`",
                            g.paths[0]
                        ),
                    }
                },
            ),
            Tab::General => self
                .report
                .clone()
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "No sentinel findings recorded. Press r to run it.".into()),
        }
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
            }
            Err(e) => self.msg = format!("not changed: {e:#}"),
        }
    }

    pub fn key(&mut self, k: KeyCode) {
        let n = self.rows().len();
        let i = self.sel[self.tab];
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
            _ if n == 0 => {}
            k => self.act(k, i),
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

    fn draw(&self, frame: &mut ratatui::Frame) {
        let [top, body, detail, foot] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Percentage(45),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .areas(frame.area());
        let titles: Vec<&str> = TABS.iter().map(|t| t.title()).collect();
        frame.render_widget(
            Tabs::new(titles)
                .select(self.tab)
                .highlight_style(Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED))
                .block(
                    Block::bordered()
                        .title(format!(" seatwise{} ", if self.dirty { " *" } else { "" })),
                ),
            top,
        );
        let items: Vec<ListItem> = self.rows().into_iter().map(ListItem::new).collect();
        let mut state = ListState::default().with_selected(Some(self.sel[self.tab]));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .block(Block::bordered().title(format!(" {} ", self.current().title()))),
            body,
            &mut state,
        );
        frame.render_widget(
            Paragraph::new(self.detail())
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(" details ")),
            detail,
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(format!(
                    "{}  ·  ←/→ tab  ↑/↓ move  w write  q quit",
                    self.current().help()
                )),
                Line::from(self.msg.clone()),
            ]),
            foot,
        );
    }
}

/// Run the screen on the config file.
pub fn run(_exe: &Path) -> Result<()> {
    let mut app = App::new(ConfigFile::open()?)?;
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        while !app.quit {
            terminal.draw(|f| app.draw(f))?;
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
    App::new(ConfigFile::open_at(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(text: &str) -> (tempfile::TempDir, App) {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.toml");
        std::fs::write(&p, text).unwrap();
        let a = app_at(&p).unwrap();
        (t, a)
    }

    const CFG: &str = "# mine\n[[seat]]\nname = \"w\"\nconfig_dir = \"~/.cw\"\n\n[[group]]\nname = \"g\"\npaths = [\"~/code\"]\npreset = \"build\"\n";

    #[test]
    fn toggles_cycle_and_write_keeping_comments() {
        let (t, mut a) = app(CFG);
        a.key(KeyCode::Char(' ')); // seat w off
        assert!(!a.cfg.seats[0].enabled);
        a.key(KeyCode::Char('e')); // enforcement global → off
        assert_eq!(a.cfg.seats[0].enforcement, Some(Enforcement::Off));
        a.key(KeyCode::Right); // agents
        a.key(KeyCode::Down);
        a.key(KeyCode::Char(' ')); // codex off
        assert!(!a.cfg.agents.codex);
        a.key(KeyCode::Right); // presets: explore first
        a.key(KeyCode::Char(' '));
        assert!(!preset::get(&a.cfg, "explore").unwrap().enabled);
        assert_eq!(
            preset::get(&a.cfg, "explore").unwrap().profiles,
            ["light"],
            "a disabled built-in keeps its content"
        );
        a.key(KeyCode::Right); // groups
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
        a.key(KeyCode::Right); // general
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
}
