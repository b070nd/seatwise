//! Moving setup between config folders without giving up isolation: copies, never links, and never a seat's
//! login, history or guard hooks.
//! - `share`: items from one shared folder copied into chosen seats on every start (skills, agents, ...).
//! - `seat add --from`: a new seat starts with a copy of another seat's setup.

use crate::config::{Config, Seat};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Copy `src` (a file or folder) to `dst`, writing only files whose content differs. Returns how many were written.
/// Files in `dst` that `src` doesn't have are left alone.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<usize> {
    let meta = fs::symlink_metadata(src).with_context(|| format!("reading {}", src.display()))?;
    if meta.is_dir() {
        let mut n = 0;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            n += copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(n);
    }
    let data = fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    if fs::read(dst).is_ok_and(|old| old == data) {
        return Ok(0);
    }
    if let Some(dir) = dst.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(dst, data).with_context(|| format!("writing {}", dst.display()))?;
    Ok(1)
}

/// Skills that tools leave in the agents' global folders (`~/.claude/skills`, `~/.agents/skills`): no seat reads them.
/// Copies each into the shared folder (as `skills/<name>`) and returns the `share` items that name them.
pub fn adopt(cfg: &Config) -> Result<Vec<String>> {
    let home = crate::paths::home();
    let mut items = Vec::new();
    for root in [home.join(".claude/skills"), home.join(".agents/skills")] {
        for entry in fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            // A skill may be a link into the tool's own folder: copy what it points at.
            let src = dunce::canonicalize(entry.path())?;
            copy_tree(&src, &cfg.shared_dir().join("skills").join(&name))?;
            let item = format!("skills/{name}");
            if !items.contains(&item) {
                items.push(item);
            }
        }
    }
    Ok(items)
}

/// Bring a seat's `share` items up to date from the shared folder. Returns notes for the user.
pub fn apply(cfg: &Config, seat: &Seat) -> Result<Vec<String>> {
    let shared = cfg.shared_dir();
    let mut notes = Vec::new();
    for item in &seat.share {
        let src = shared.join(item);
        if !src.exists() {
            notes.push(format!(
                "shared item '{item}' is missing from {}",
                shared.display()
            ));
            continue;
        }
        let n = copy_tree(&src, &seat.config_dir().join(item))?;
        if n > 0 {
            notes.push(format!(
                "updated {n} file(s) of shared '{item}' for seat '{}'",
                seat.name
            ));
        }
    }
    Ok(notes)
}

/// What `seat add --from` copies: setup, not identity. No login, history, sessions or caches.
pub const SETUP_ITEMS: [&str; 7] = [
    "settings.json",
    "CLAUDE.md",
    "skills",
    "agents",
    "commands",
    "output-styles",
    "plugins",
];

/// Copy `from`'s setup into `to`'s config folder. Paths that point into `from`'s folder (plugin install
/// paths, a status line script) are rewritten to `to`'s, and `from`'s guard hooks are dropped.
pub fn copy_setup(from: &Seat, to: &Seat) -> Result<Vec<String>> {
    let (src, dst) = (from.config_dir(), to.config_dir());
    let mut copied = Vec::new();
    for item in SETUP_ITEMS {
        if src.join(item).exists() {
            copy_tree(&src.join(item), &dst.join(item))?;
            copied.push(item.to_string());
        }
    }
    let rewrite = |file: &Path| -> Result<()> {
        let Ok(text) = fs::read_to_string(file) else {
            return Ok(());
        };
        let mut out = text.clone();
        for (a, b) in path_spellings(&src).into_iter().zip(path_spellings(&dst)) {
            out = out.replace(&a, &b);
        }
        if out != text {
            fs::write(file, out)?;
        }
        Ok(())
    };
    rewrite(&dst.join("settings.json"))?;
    if let Ok(rd) = fs::read_dir(dst.join("plugins")) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                rewrite(&e.path())?;
            }
        }
    }
    let settings = dst.join("settings.json");
    if let Ok(text) = fs::read_to_string(&settings)
        && let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let before = v.clone();
        crate::claude::remove_hooks(&mut v);
        if v != before {
            fs::write(&settings, serde_json::to_string_pretty(&v)? + "\n")?;
        }
    }
    Ok(copied)
}

/// A folder path as it may appear in Claude Code's files: plain, and JSON-escaped (Windows backslashes).
fn path_spellings(p: &Path) -> Vec<String> {
    let plain = p.display().to_string();
    let escaped = plain.replace('\\', "\\\\");
    if escaped == plain {
        vec![plain]
    } else {
        vec![escaped, plain]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(name: &str, dir: &Path) -> Seat {
        Seat {
            name: name.into(),
            config_dir: dir.display().to_string(),
            ..Seat::default()
        }
    }

    #[test]
    fn copies_only_what_changed() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("a/b/x.md"), "x").unwrap();
        fs::write(src.join("a/y.md"), "y").unwrap();
        fs::create_dir_all(dst.join("a")).unwrap();
        fs::write(dst.join("a/own.md"), "mine").unwrap();
        assert_eq!(copy_tree(&src.join("a"), &dst.join("a")).unwrap(), 2);
        assert_eq!(copy_tree(&src.join("a"), &dst.join("a")).unwrap(), 0);
        assert_eq!(fs::read_to_string(dst.join("a/own.md")).unwrap(), "mine");
    }

    #[test]
    fn setup_copy_rewrites_paths_and_drops_hooks() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join(".claude-a"), t.path().join(".claude-b"));
        fs::create_dir_all(a.join("plugins")).unwrap();
        fs::create_dir_all(a.join("projects/p")).unwrap();
        fs::write(a.join(".claude.json"), "{}").unwrap();
        fs::write(a.join("projects/p/s.jsonl"), "{}").unwrap();
        let settings = serde_json::json!({
            "model": "m",
            "statusLine": { "command": format!("bash {}/status.sh", a.display()) },
            "hooks": { "PreToolUse": [{ "matcher": "Read", "hooks": [{ "type": "command", "command": "\"/bin/seat\" hook tool a" }] }] }
        });
        fs::write(a.join("settings.json"), settings.to_string()).unwrap();
        fs::write(
            a.join("plugins/installed_plugins.json"),
            serde_json::json!({ "p": { "installPath": a.join("plugins/cache/p") } }).to_string(),
        )
        .unwrap();
        let copied = copy_setup(&seat("a", &a), &seat("b", &b)).unwrap();
        assert_eq!(copied, ["settings.json", "plugins"]);
        assert!(!b.join(".claude.json").exists(), "login is not copied");
        assert!(!b.join("projects").exists(), "history is not copied");
        let s = fs::read_to_string(b.join("settings.json")).unwrap();
        assert!(
            s.contains(".claude-b/status.sh") && !s.contains(".claude-a"),
            "{s}"
        );
        assert!(!s.contains("hook tool"), "seat a's hooks are dropped: {s}");
        let p = fs::read_to_string(b.join("plugins/installed_plugins.json")).unwrap();
        assert!(p.contains(".claude-b") && !p.contains(".claude-a"), "{p}");
    }
}
