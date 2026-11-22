//! A home repo per seat (the "home repo" pattern): one git repo where the seat's sessions start. It holds the
//! seat's shared skills, its AGENTS.md and docs, and pointers to the seat's other repos, so one session can work
//! across all of them.
//!
//! seatwise keeps it per seat: the pointer list holds only that seat's repos, generated from its folders, so a
//! home repo never leads a session into another seat's work.
//!
//! Layout:
//! - `AGENTS.md`: instructions for every agent, with a managed "Other repos" block.
//! - `CLAUDE.md`: `@AGENTS.md` (Claude Code reads CLAUDE.md, not AGENTS.md).
//! - `skills/`: copied into a Claude Code seat's own skills on every start, so they work in all its projects.
//! - `.agents/skills/`: Codex finds project skills here when a session starts in the repo.
//! - `docs/`: coding standards, glossary, decisions.

use crate::config::{Agent, Config, Owner, Seat};
use crate::folders::{key_string, owner_of};
use crate::paths;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const START: &str = "<!-- seatwise:repos:start -->";
const END: &str = "<!-- seatwise:repos:end -->";

pub fn default_path(seat: &Seat) -> PathBuf {
    paths::home().join(format!("{}-home", seat.name))
}

fn write_new(path: &Path, text: &str) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Create the home repo's files (never overwriting) and `git init` it if git is there. Returns what was created.
pub fn scaffold(seat: &Seat, dir: &Path) -> Result<Vec<String>> {
    let mut made = Vec::new();
    fs::create_dir_all(dir)?;
    let agents = format!(
        "# {name} home\n\n\
         Start sessions for the `{name}` seat here. This repo holds the seat's shared skills, instructions and docs,\n\
         and points to its other repos below. Open or clone one of them when the work needs it.\n\n\
         Only this seat's repos are listed. Don't work in folders that aren't listed: they may belong to another seat.\n\n\
         {START}\n{END}\n",
        name = seat.name
    );
    let mut files: Vec<(&str, String)> = vec![
        ("AGENTS.md", agents),
        (
            "docs/README.md",
            "# Docs\n\nShared standards, glossary and decisions for this seat's repos.\n".into(),
        ),
    ];
    match seat.agent {
        Agent::Claude => {
            files.push(("CLAUDE.md", "@AGENTS.md\n".into()));
            files.push(("skills/.gitkeep", String::new()));
        }
        Agent::Codex => files.push((".agents/skills/.gitkeep", String::new())),
    }
    for (name, text) in files {
        if write_new(&dir.join(name), &text)? {
            made.push(name.to_string());
        }
    }
    if !dir.join(".git").exists()
        && Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success())
    {
        made.push("git repo".into());
    }
    Ok(made)
}

fn git_remote(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output()
        .ok()?;
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !url.is_empty()).then_some(url)
}

/// First heading or line of the README, as a short description.
fn describe(dir: &Path) -> Option<String> {
    let text = ["README.md", "readme.md", "README"]
        .iter()
        .find_map(|f| fs::read_to_string(dir.join(f)).ok())?;
    let line = text
        .lines()
        .map(|l| l.trim_start_matches('#').trim())
        .find(|l| !l.is_empty() && !l.starts_with('<') && !l.starts_with('!'))?;
    Some(line.chars().take(80).collect())
}

/// The repos a seat's home repo points to: each of its folders that is a git repo, or the git repos directly
/// inside a folder that isn't one. Repos owned by another seat (a nested assignment) are left out.
pub fn repos(cfg: &Config, seat: &Seat) -> Vec<PathBuf> {
    let home = seat.home().map(|h| key_string(&h));
    let mut out = Vec::new();
    for folder in cfg.folders_of(&seat.name) {
        if !folder.is_dir() {
            continue;
        }
        let candidates: Vec<PathBuf> = if folder.join(".git").exists() {
            vec![folder]
        } else {
            let mut kids: Vec<PathBuf> = fs::read_dir(&folder)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join(".git").exists())
                .collect();
            kids.sort();
            kids
        };
        for c in candidates {
            let mine = owner_of(cfg, &crate::folders::normalize(&c, &paths::home()))
                == Some(Owner::Seat(&seat.name));
            if mine && Some(key_string(&c)) != home && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// The managed "Other repos" block, in the pointer format: `- name (what it is): path. Clone from <remote> if missing.`
pub fn pointer_block(cfg: &Config, seat: &Seat) -> String {
    let mut block = format!(
        "{START}\n## Other repos\n\n<!-- Managed by seatwise from the {} seat's folders: `seat home sync {}`. -->\n\n",
        seat.name, seat.name
    );
    let repos = repos(cfg, seat);
    if repos.is_empty() {
        block += "None yet: give folders to this seat with `seat link`.\n";
    }
    for r in repos {
        let name = r
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let what = describe(&r).map(|d| format!(" ({d})")).unwrap_or_default();
        let clone = git_remote(&r)
            .map(|u| format!(" Clone from {u} if missing."))
            .unwrap_or_default();
        block += &format!("- {name}{what}: {}.{clone}\n", paths::contract(&r));
    }
    block + END
}

/// Rewrite the managed block in the home repo's AGENTS.md (added at the end if missing).
pub fn sync(cfg: &Config, seat: &Seat) -> Result<PathBuf> {
    let dir = seat.home().with_context(|| {
        format!(
            "seat '{}' has no home repo (`seat home init {}`)",
            seat.name, seat.name
        )
    })?;
    let file = dir.join("AGENTS.md");
    let text = fs::read_to_string(&file).unwrap_or_default();
    let block = pointer_block(cfg, seat);
    let new = match (text.find(START), text.find(END)) {
        (Some(a), Some(b)) if b > a => format!("{}{block}{}", &text[..a], &text[b + END.len()..]),
        _ => format!(
            "{}{}{block}\n",
            text,
            if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                "\n"
            }
        ),
    };
    if new != text {
        fs::write(&file, new).with_context(|| format!("writing {}", file.display()))?;
    }
    Ok(file)
}

/// Claude Code seats: copy the home repo's `skills/` into the seat's own skills (on every start).
pub fn apply_skills(seat: &Seat) -> Result<Vec<String>> {
    let Some(home) = seat.home() else {
        return Ok(vec![]);
    };
    if seat.agent != Agent::Claude || !home.join("skills").is_dir() {
        return Ok(vec![]);
    }
    let n = crate::share::copy_tree(&home.join("skills"), &seat.config_dir().join("skills"))?;
    Ok(if n > 0 {
        vec![format!(
            "updated {n} skill file(s) from the home repo of seat '{}'",
            seat.name
        )]
    } else {
        vec![]
    })
}

/// Paths of other seats' folders that the home repo's instructions mention: they would lead this seat's sessions
/// into another seat's work.
pub fn foreign_mentions(cfg: &Config, seat: &Seat) -> Vec<String> {
    let Some(home) = seat.home() else {
        return vec![];
    };
    let text = ["AGENTS.md", "CLAUDE.md"]
        .iter()
        .filter_map(|f| fs::read_to_string(home.join(f)).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    for other in cfg.seats.iter().filter(|s| s.name != seat.name) {
        for f in cfg.folders_of(&other.name) {
            let short = paths::contract(&f);
            if text.contains(&short) || text.contains(&f.display().to_string()) {
                out.push(format!("{short} (seat '{}')", other.name));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_block_is_replaced_in_place() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("h");
        let seat = Seat {
            name: "w".into(),
            config_dir: t.path().join(".c").display().to_string(),
            home: Some(home.display().to_string()),
            ..Seat::default()
        };
        let cfg = Config {
            seats: vec![seat.clone()],
            ..Config::default()
        };
        let made = scaffold(&seat, &home).unwrap();
        assert!(made.contains(&"AGENTS.md".to_string()) && made.contains(&"CLAUDE.md".to_string()));
        fs::write(
            home.join("AGENTS.md"),
            fs::read_to_string(home.join("AGENTS.md")).unwrap() + "\n## Mine\nkeep me\n",
        )
        .unwrap();
        sync(&cfg, &seat).unwrap();
        sync(&cfg, &seat).unwrap();
        let text = fs::read_to_string(home.join("AGENTS.md")).unwrap();
        assert_eq!(text.matches(START).count(), 1, "{text}");
        assert!(
            text.contains("## Other repos") && text.contains("keep me"),
            "{text}"
        );
        assert_eq!(
            fs::read_to_string(home.join("CLAUDE.md")).unwrap(),
            "@AGENTS.md\n"
        );
    }
}
