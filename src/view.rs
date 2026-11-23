//! The Seat view and the Project view: everything about one seat, or one project folder, gathered in one value.
//!
//! Screens render these instead of collecting facts themselves: the TUI draws them as cards and tables,
//! `seat show` and `seat projects` print them as text or JSON. Each fact carries a [`Health`] so every screen colors
//! and orders the same way.

use crate::config::{Agent, Apply, Config, Owner, Seat};
use crate::status::{Identity, SeatStatus};
use crate::{catalog, paths, preset};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Bad,
    Notice,
    Good,
    /// Not in use (a disabled seat, a feature that's off): neither good nor bad.
    Off,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub label: &'static str,
    pub value: String,
    pub health: Health,
}

fn fact(label: &'static str, value: impl Into<String>, health: Health) -> Fact {
    Fact {
        label,
        value: value.into(),
        health,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SeatView {
    pub name: String,
    pub aliases: Vec<String>,
    pub agent: &'static str,
    pub enabled: bool,
    /// The worst health among the facts that matter (identity, hooks); `Off` when disabled.
    pub health: Health,
    /// One or two words for lists: `ok`, `signed out`, `wrong org`, `hooks missing`, `off`.
    pub status: String,
    pub facts: Vec<Fact>,
    pub projects: Vec<ProjectView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub path: String,
    pub exists: bool,
    /// The seat that owns it, `any` (a shared start folder) or `none` (closed to every seat).
    pub owner: String,
    pub group: Option<String>,
    /// The preset sessions here get, or `-`.
    pub preset: String,
    pub apply: &'static str,
    /// Sessions recorded for this folder in the owner's config folder.
    pub sessions: usize,
    /// When the newest of them was last written, as "3d ago".
    pub last_used: Option<String>,
    /// Problems the settings catalog finds in its preset where it is applied.
    pub preset_problems: Vec<String>,
    pub health: Health,
}

fn ago(t: SystemTime) -> String {
    let s = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86_400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// Session count and newest session time for a folder under a seat.
fn sessions(seat: &Seat, dir: &Path) -> (usize, Option<SystemTime>) {
    let mut n = 0;
    let mut newest = None;
    for e in std::fs::read_dir(crate::sentinel::sessions_dir(seat, dir))
        .into_iter()
        .flatten()
        .flatten()
    {
        if e.path().extension().is_some_and(|x| x == "jsonl") {
            n += 1;
            if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                newest = newest.max(Some(t));
            }
        }
    }
    (n, newest)
}

fn project(cfg: &Config, dir: PathBuf, owner: Owner<'_>) -> ProjectView {
    let exists = dir.is_dir();
    let (owner_label, seat) = match owner {
        Owner::Seat(s) => (s.to_string(), cfg.seat(s)),
        Owner::Any => ("any".to_string(), None),
        Owner::Nobody => ("none".to_string(), None),
    };
    let group = preset::group_for(cfg, &dir);
    let apply = group.and_then(|g| g.apply).unwrap_or(cfg.preset_apply);
    let preset_name = group
        .map(|g| g.preset.clone())
        .filter(|p| p != crate::config::NONE);
    let preset_problems: Vec<String> = preset_name
        .as_deref()
        .and_then(|p| preset::get(cfg, p))
        .map(|p| {
            let target = if apply == Apply::Project {
                catalog::Target::Local
            } else {
                catalog::Target::Session
            };
            catalog::check(&preset::settings(cfg, &p), target)
                .into_iter()
                .filter(|p| p.level != catalog::Level::Info)
                .map(|p| p.message)
                .collect()
        })
        .unwrap_or_default();
    let (n, newest) = seat.map_or((0, None), |s| sessions(s, &dir));
    let health = if !exists {
        Health::Notice
    } else if !preset_problems.is_empty() {
        Health::Bad
    } else if seat.is_none() {
        Health::Off
    } else {
        Health::Good
    };
    ProjectView {
        path: paths::contract(&dir),
        exists,
        owner: owner_label,
        group: group.map(|g| g.name.clone()),
        preset: preset_name.unwrap_or_else(|| "-".into()),
        apply: apply.as_str(),
        sessions: n,
        last_used: newest.map(ago),
        preset_problems,
        health,
    }
}

/// Every configured folder as a project, most recently used first.
pub fn projects(cfg: &Config) -> Vec<ProjectView> {
    let mut out: Vec<(Option<SystemTime>, ProjectView)> = cfg
        .folders
        .iter()
        .map(|f| {
            let dir = paths::expand(&f.path);
            let newest = match f.owner() {
                Owner::Seat(s) => cfg.seat(s).and_then(|s| sessions(s, &dir).1),
                _ => None,
            };
            (newest, project(cfg, dir, f.owner()))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
    out.into_iter().map(|(_, p)| p).collect()
}

fn identity_health(i: &Identity) -> Health {
    match i {
        Identity::Ok => Health::Good,
        Identity::Unpinned | Identity::SignedOut => Health::Notice,
        Identity::LoggedOut | Identity::Wrong(_) => Health::Bad,
    }
}

/// Everything about one seat. `desktop` is the running desktop instances (`desktop::running`), looked up once by the
/// caller for all seats.
pub fn seat(
    cfg: &Config,
    seat: &Seat,
    exe: &Path,
    desktop: &[crate::desktop::Instance],
) -> SeatView {
    let st = SeatStatus::of(cfg, seat, exe);
    let enabled = cfg.is_enabled(seat);
    let id_health = identity_health(&st.identity);
    let hooks_health = if st.hooks_ok() {
        if st.enforcement == crate::config::Enforcement::Off {
            Health::Off
        } else {
            Health::Good
        }
    } else if st.enforcement == crate::config::Enforcement::Off {
        Health::Notice
    } else {
        Health::Bad
    };
    let mut facts = vec![
        fact("account", st.account_label(), id_health),
        fact("plan", st.plan_label(), Health::Good),
        fact(
            "identity",
            match &st.identity {
                Identity::Ok => "matches its pin".to_string(),
                Identity::Unpinned => "not pinned (`seat pin`)".to_string(),
                Identity::SignedOut => "signed out (`seat login`)".to_string(),
                Identity::LoggedOut => "login gone (`seat login`)".to_string(),
                Identity::Wrong(_) => "signed into the wrong account".to_string(),
            },
            id_health,
        ),
        fact(
            "isolation",
            format!(
                "enforcement {}, hooks {}",
                st.enforcement.as_str(),
                st.hooks_label()
            ),
            hooks_health,
        ),
        fact("links", st.link_label(), Health::Good),
        fact(
            "profile",
            seat.profile.clone().unwrap_or_else(|| "-".into()),
            if seat.profile.is_some() {
                Health::Good
            } else {
                Health::Off
            },
        ),
        fact("config", paths::contract(&seat.config_dir()), Health::Good),
    ];
    if seat.agent == Agent::Claude && cfg!(target_os = "macos") {
        let mine = desktop
            .iter()
            .find(|i| i.seat.as_deref() == Some(seat.name.as_str()));
        facts.push(match mine {
            Some(i) => fact(
                "desktop",
                format!("running since {}", i.started),
                Health::Good,
            ),
            None => fact(
                "desktop",
                format!("not running (`seat desktop {}`)", seat.name),
                Health::Off,
            ),
        });
    }
    let (health, status) = if !enabled {
        (Health::Off, "off".to_string())
    } else {
        let worst = id_health.min(hooks_health);
        let status = match (&st.identity, worst) {
            (Identity::Wrong(_), _) => "wrong account",
            (Identity::LoggedOut, _) | (Identity::SignedOut, _) => "signed out",
            (_, Health::Bad) if st.hooks == crate::claude::HookState::Stale => "hooks outdated",
            (_, Health::Bad) => "hooks missing",
            (Identity::Unpinned, _) => "not pinned",
            _ => "ok",
        };
        (worst.min(Health::Good), status.to_string())
    };
    let projects = cfg
        .folders_of(&seat.name)
        .into_iter()
        .map(|dir| project(cfg, dir, Owner::Seat(&seat.name)))
        .collect();
    SeatView {
        name: seat.name.clone(),
        aliases: seat.aliases.clone(),
        agent: seat.agent.as_str(),
        enabled,
        health,
        status,
        facts,
        projects,
    }
}

/// Every seat, in config order.
pub fn seats(cfg: &Config, exe: &Path) -> Vec<SeatView> {
    let desktop = crate::desktop::running(cfg);
    cfg.seats
        .iter()
        .map(|s| seat(cfg, s, exe, &desktop))
        .collect()
}

/// A seat as text, for `seat show`.
pub fn seat_text(v: &SeatView) -> String {
    let mut s = format!("{} · {}", v.name, v.agent);
    if !v.aliases.is_empty() {
        s += &format!(" ({})", v.aliases.join(", "));
    }
    s += &format!("  [{}]\n", v.status);
    for f in &v.facts {
        s += &format!("  {:<10} {}\n", f.label, f.value);
    }
    if !v.projects.is_empty() {
        s += &format!("\n  projects ({})\n", v.projects.len());
        s += &projects_text(&v.projects, false);
    }
    s
}

/// A path shortened in the middle to `max` characters: `~/code/very/…/deep/app`.
pub fn short_path(p: &str, max: usize) -> String {
    let n = p.chars().count();
    if n <= max {
        return p.to_string();
    }
    let keep = max.saturating_sub(1);
    let head: String = p.chars().take(keep / 2).collect();
    let tail: String = p.chars().skip(n - (keep - keep / 2)).collect();
    format!("{head}…{tail}")
}

/// Projects as an aligned table.
pub fn projects_text(projects: &[ProjectView], with_owner: bool) -> String {
    let mut rows = vec![{
        let mut h = vec!["PATH".to_string()];
        if with_owner {
            h.push("SEAT".into());
        }
        h.extend(["PRESET", "SESSIONS", "LAST USED", ""].map(String::from));
        h
    }];
    for p in projects {
        let mut r = vec![short_path(&p.path, 56)];
        if with_owner {
            r.push(p.owner.clone());
        }
        let preset = if p.preset == "-" {
            "-".to_string()
        } else {
            format!("{} ({})", p.preset, p.apply)
        };
        let note = if !p.exists {
            "missing".to_string()
        } else if !p.preset_problems.is_empty() {
            format!("preset: {} problem(s)", p.preset_problems.len())
        } else {
            String::new()
        };
        r.extend([
            preset,
            p.sessions.to_string(),
            p.last_used.clone().unwrap_or_else(|| "-".into()),
            note,
        ]);
        rows.push(r);
    }
    crate::status::table(&rows)
        .lines()
        .map(|l| format!("  {l}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_knows_its_owner_preset_and_sessions() {
        let t = tempfile::tempdir().unwrap();
        let home = dunce::canonicalize(t.path()).unwrap();
        let app = home.join("code/app");
        std::fs::create_dir_all(&app).unwrap();
        let cfg: Config = toml::from_str(&format!(
            "[[seat]]\nname = \"w\"\nconfig_dir = '{}'\n[[folder]]\npath = '{}'\nowner = \"w\"\n\
             [[group]]\nname = \"g\"\npaths = ['{}']\npreset = \"build\"\n",
            home.join(".cw").display(),
            app.display(),
            app.display()
        ))
        .unwrap();
        let seat = cfg.seat("w").unwrap();
        let dir = crate::sentinel::sessions_dir(seat, &app);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), "{}").unwrap();
        let p = &projects(&cfg)[0];
        assert_eq!(
            (p.owner.as_str(), p.preset.as_str(), p.sessions),
            ("w", "build", 1)
        );
        assert_eq!(p.last_used.as_deref(), Some("just now"));
        assert_eq!(p.health, Health::Good);
        let v = seat_text(&super::seat(&cfg, seat, Path::new("/x/seat"), &[]));
        assert!(
            v.contains("w · claude") && v.contains("build (session)"),
            "{v}"
        );
    }
}
