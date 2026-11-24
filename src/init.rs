//! `seat init`: find the Claude Code config folders already on this machine, who each is signed in as, and
//! which project folders each has been used in, and propose a config from that.

use crate::claude::{self, Account};
use crate::folders::{key_string, normalize};
use crate::paths;
use std::collections::{BTreeMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Found {
    pub name: String,
    pub config_dir: PathBuf,
    pub account: Option<Account>,
    /// Folders its sessions ran in, most-nested duplicates removed.
    pub folders: Vec<PathBuf>,
    /// A Codex home (`.codex*`), not a Claude Code folder.
    pub codex: bool,
    /// The Claude desktop app runs in its default data folder for this config folder, so that folder holds this
    /// seat's desktop login.
    pub desktop_default: bool,
}

fn is_codex(dir: &Path) -> bool {
    dir.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with(".codex"))
}

/// Config folders in the home folder (`.claude`, `.claude-*`) and `$CLAUDE_CONFIG_DIR`.
/// Config folders that look like an agent's, by path.
fn candidate_dirs() -> Vec<PathBuf> {
    let home = paths::home();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n == ".claude"
                || n.starts_with(".claude-")
                || n == ".codex"
                || n.starts_with(".codex-"))
                && e.path().is_dir()
        })
        .map(|e| e.path())
        .collect();
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
        && d.is_dir()
    {
        dirs.push(d);
    }
    dirs.sort();
    dirs
}

/// The state file holding the account signed into a config folder.
fn state_file(dir: &Path) -> PathBuf {
    let home = paths::home();
    if is_codex(dir) {
        dir.join("auth.json")
    } else if dir == home.join(".claude") {
        home.join(".claude.json")
    } else {
        dir.join(".claude.json")
    }
}

/// The account in every config folder on this machine, with the name `init` would give its seat. Cheap: no
/// session history is read.
pub fn accounts() -> Vec<(String, claude::Account)> {
    candidate_dirs()
        .into_iter()
        .filter(|d| looks_like_config(d))
        .filter_map(|d| {
            let state = state_file(&d);
            let account = if is_codex(&d) {
                claude::read_codex_account(&state)
            } else {
                claude::read_account_from(&state)
            }?;
            Some((seat_name(&d), account))
        })
        .collect()
}

pub fn discover() -> Vec<Found> {
    let dirs = candidate_dirs();
    let mut seen = HashSet::new();
    let mut names = HashSet::new();
    let mut out = Vec::new();
    let desktop = crate::desktop::running_default_app();
    for dir in dirs {
        if !seen.insert(key_string(&dir)) || !looks_like_config(&dir) {
            continue;
        }
        let codex = is_codex(&dir);
        let state = state_file(&dir);
        let mut name = seat_name(&dir);
        let base = name.clone();
        let mut i = 2;
        while !names.insert(name.clone()) {
            name = format!("{base}-{i}");
            i += 1;
        }
        out.push(Found {
            name,
            folders: if codex { vec![] } else { session_folders(&dir) },
            account: if codex {
                claude::read_codex_account(&state)
            } else {
                claude::read_account_from(&state)
            },
            codex,
            desktop_default: !codex && crate::desktop::on_default_data_dir(&desktop, &dir),
            config_dir: dir,
        });
    }
    out
}

/// The agent's own files, not just a folder with a similar name.
fn looks_like_config(dir: &Path) -> bool {
    dir.join("auth.json").is_file()
        || dir.join("projects").is_dir()
        || [".claude.json", ".credentials.json", "settings.json"]
            .iter()
            .any(|f| dir.join(f).is_file())
}

/// Three or more sibling folders with the same users become their parent (e.g. one scratch folder per session).
/// Returns the folders and, of those, the parents that stand in for their subfolders (guesses to check).
fn collapse_siblings(
    mut owners: BTreeMap<PathBuf, Vec<String>>,
) -> (BTreeMap<PathBuf, Vec<String>>, HashSet<PathBuf>) {
    let home = paths::home();
    let mut guessed = HashSet::new();
    loop {
        let mut groups: BTreeMap<(PathBuf, Vec<String>), Vec<PathBuf>> = BTreeMap::new();
        for (path, seats) in &owners {
            if let Some(parent) = path.parent()
                && parent != home
                && parent.parent().is_some()
            {
                groups
                    .entry((parent.to_path_buf(), seats.clone()))
                    .or_default()
                    .push(path.clone());
            }
        }
        let Some(((parent, seats), members)) = groups.into_iter().find(|(_, m)| m.len() >= 3)
        else {
            return (owners, guessed);
        };
        for m in members {
            owners.remove(&m);
            guessed.remove(&m);
        }
        guessed.insert(parent.clone());
        owners.entry(parent).or_insert(seats);
    }
}

/// `.claude` → `default`, `.claude-work` → `work`; other characters become `-`.
pub fn seat_name(dir: &Path) -> String {
    let file = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let raw = file
        .strip_prefix(".codex-")
        .or_else(|| file.strip_prefix(".claude-"))
        .or_else(|| file.strip_prefix('.'))
        .unwrap_or(&file);
    let raw = match raw {
        "claude" => "default",
        "codex" => "codex",
        r => r,
    };
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    match clean.as_str() {
        "" | "any" | "none" | "auto" => format!("seat-{clean}"),
        _ => clean,
    }
}

/// The working folders recorded in a config folder's session logs (`projects/*/*.jsonl`, the `cwd` field).
pub fn session_folders(dir: &Path) -> Vec<PathBuf> {
    let home = paths::home();
    let mut found: Vec<PathBuf> = Vec::new();
    for project in std::fs::read_dir(dir.join("projects"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let newest = std::fs::read_dir(project.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
        if let Some(cwd) = newest.and_then(|e| first_cwd(&e.path())) {
            let p = normalize(Path::new(&cwd), &home);
            if p != home && p.is_dir() && !p.starts_with(dir) && !transient(&p, &home) {
                found.push(p);
            }
        }
    }
    found.sort();
    found.dedup();
    // Keep only the outermost: a session in ~/code/app/sub is covered by ~/code/app.
    let keep: Vec<PathBuf> = found
        .iter()
        .filter(|p| !found.iter().any(|q| q != *p && p.starts_with(q)))
        .cloned()
        .collect();
    keep
}

/// Folders an app makes for itself rather than projects: temp folders, per-session scratch folders (the desktop
/// app's `scratch-workspaces`, say) and app data (`~/Library`, `~/.cache`, `~/.local/share`, `AppData`). Sessions
/// there say nothing about which seat owns which project.
fn transient(p: &Path, home: &Path) -> bool {
    let temp = [
        std::env::temp_dir(),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/private/var/folders"),
        PathBuf::from("/var/folders"),
    ];
    let app_data = [
        "Library",
        ".cache",
        ".local/share",
        ".local/state",
        "AppData",
    ];
    // Temp folders count only outside home (a home inside temp, as in tests, keeps its projects).
    (!p.starts_with(home) && temp.iter().any(|t| p.starts_with(t)))
        || app_data.iter().any(|d| p.starts_with(home.join(d)))
        || p.components().any(|c| {
            let c = c.as_os_str().to_string_lossy().to_ascii_lowercase();
            c.starts_with("scratch-workspace") || c == "scratchpad"
        })
}

fn first_cwd(log: &Path) -> Option<String> {
    let file = std::fs::File::open(log).ok()?;
    std::io::BufReader::new(file)
        .lines()
        .take(200)
        .map_while(Result::ok)
        .find_map(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()?
                .get("cwd")?
                .as_str()
                .map(str::to_owned)
        })
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// A config.toml for what was found, with comments where a decision is the user's.
pub fn propose(found: &[Found]) -> String {
    let mut out = String::from(
        "# seatwise config, proposed by `seat init` from the Claude Code folders on this machine.\n\
         # Review it, then check with `seat doctor`. All options: config.example.toml.\n\n\
         # The config format; seatwise refuses a config from a newer version instead of guessing.\n\
         version = 1\n\n\
         # Isolation between seats: \"warn\" (recommended) tells you and asks before a session touches another\n\
         # seat's folders; \"strict\" blocks it; \"off\" only picks the seat by folder. Hooks are installed on the\n\
         # next start (or `seat install-hooks`).\n\
         enforcement = \"warn\"\n",
    );
    // An agent's default folder next to named ones is usually left over from before seats: proposed commented out.
    let home = paths::home();
    let legacy: HashSet<&str> = found
        .iter()
        .filter(|f| {
            let default = home.join(if f.codex { ".codex" } else { ".claude" });
            f.config_dir == default
                && found
                    .iter()
                    .any(|g| g.codex == f.codex && g.config_dir != default)
        })
        .map(|f| f.name.as_str())
        .collect();
    for f in found {
        let start = out.len();
        out += &format!(
            "\n[[seat]]\nname = {}\nconfig_dir = {}\n",
            quote(&f.name),
            quote(&paths::contract(&f.config_dir))
        );
        if f.codex {
            out += "agent = \"codex\"\n";
        }
        match &f.account {
            Some(a) if a.org_type.as_deref() == Some(claude::API_KEY) && a.email.is_none() => {
                out += "# signed in with an API key: no account to pin\n";
            }
            Some(a) => {
                if let Some(o) = &a.org_uuid {
                    out += &format!("org_uuid = {}\n", quote(o));
                }
                if let Some(o) = &a.org_name {
                    out += &format!("org_name = {}\n", quote(o));
                }
                if let Some(e) = &a.email {
                    out += &format!("email = {}\n", quote(e));
                }
            }
            None => {
                out += &format!(
                    "# not signed in: `seat login {}`, then `seat pin {}`\n",
                    f.name, f.name
                )
            }
        }
        // Only one seat can have the default data folder.
        if f.desktop_default
            && found
                .iter()
                .find(|g| g.desktop_default)
                .is_some_and(|g| g.name == f.name)
        {
            out += "# The desktop app runs in its default data folder for this seat: keep its desktop login there.\n\
                    [seat.desktop]\nuser_data_dir = \"default\"\n";
        }
        if legacy.contains(f.name.as_str()) {
            let block: String = out[start..]
                .lines()
                .map(|l| {
                    if l.is_empty() || l.starts_with('#') {
                        format!("{l}\n")
                    } else {
                        format!("# {l}\n")
                    }
                })
                .collect();
            out.truncate(start);
            out += &format!(
                "\n# {} is the agent's default folder, next to seats with folders of their own: usually left over from\n\
                 # before them. Not proposed as a seat; uncomment to keep it as one.{}",
                paths::contract(&f.config_dir),
                format_args!("\n{}", block.trim_start_matches('\n'))
            );
        } else if f.config_dir == home.join(".claude") && found.len() > 1 {
            out += "# This seat lives in ~/.claude: its CLAUDE.md is loaded into every seat's sessions under your\n\
                    # home folder. Consider moving it to its own folder (`seat add <name> --from default`).\n";
        }
    }
    let mut owners: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for f in found {
        for p in &f.folders {
            owners.entry(p.clone()).or_default().push(f.name.clone());
        }
    }
    for seats in owners.values_mut() {
        seats.retain(|s| !legacy.contains(s.as_str()));
    }
    owners.retain(|_, seats| !seats.is_empty());
    let (owners, guessed) = collapse_siblings(owners);
    if !owners.is_empty() {
        out +=
            "\n# Folders, from where each seat's sessions ran. The longest matching path wins.\n";
    }
    for (path, seats) in owners {
        let shown = quote(&paths::contract(&path));
        let note = if guessed.contains(&path) {
            "\n# Guessed: sessions ran in several of its subfolders, so the whole folder is proposed. Check it."
        } else {
            ""
        };
        if let [only] = seats.as_slice() {
            out += &format!(
                "{note}\n[[folder]]\npath = {shown}\nowner = {}\n",
                quote(only)
            );
        } else {
            out += &format!(
                "\n# Used by {}: pick the owner, or make it a shared start folder (\"any\").\n# [[folder]]\n# path = {shown}\n# owner = \"\"\n",
                seats.join(" and ")
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_and_app_folders_are_not_projects() {
        let home = Path::new("/h");
        for p in [
            "/h/Library/Application Support/App/scratch-workspaces/s1",
            "/h/.cache/x",
            "/h/AppData/Roaming/x",
            "/tmp/x",
            "/h/work/scratch-workspaces/a",
        ] {
            assert!(transient(Path::new(p), home), "{p}");
        }
        for p in ["/h/code/app", "/h/Libraryish/app", "/h/scratch/app"] {
            assert!(!transient(Path::new(p), home), "{p}");
        }
    }

    #[test]
    fn names_from_folders() {
        assert_eq!(seat_name(Path::new("/h/.claude")), "default");
        assert_eq!(seat_name(Path::new("/h/.claude-Work")), "work");
        assert_eq!(seat_name(Path::new("/h/.claude-a b")), "a-b");
        assert_eq!(seat_name(Path::new("/h/.claude-any")), "seat-any");
    }

    #[test]
    fn proposal_is_a_valid_config() {
        let t = tempfile::tempdir().unwrap();
        let root = normalize(t.path(), Path::new("/"));
        let (shared, mine) = (root.join("shared"), root.join("mine"));
        let found = vec![
            Found {
                name: "work".into(),
                config_dir: root.join(".claude-work"),
                account: Some(Account {
                    email: Some("me@corp.example".into()),
                    org_uuid: Some("org-1".into()),
                    ..Account::default()
                }),
                folders: vec![shared.clone()],
                codex: false,
                desktop_default: true,
            },
            Found {
                name: "home".into(),
                config_dir: root.join(".claude-home"),
                account: None,
                folders: vec![shared, mine],
                codex: false,
                desktop_default: true,
            },
        ];
        let text = propose(&found);
        let cfg: crate::config::Config = toml::from_str(&text).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.seats.len(), 2);
        assert_eq!(
            crate::desktop::data_dir(cfg.seat("work").unwrap()),
            None,
            "the seat whose desktop runs in the default folder keeps it: {text}"
        );
        assert!(
            crate::desktop::data_dir(cfg.seat("home").unwrap()).is_some(),
            "only one seat gets the default folder"
        );
        assert_eq!(
            cfg.seat("work").unwrap().email.as_deref(),
            Some("me@corp.example")
        );
        assert_eq!(
            cfg.folders.len(),
            1,
            "the folder both used is left to the user: {text}"
        );
        assert_eq!(cfg.folders[0].owner, "home");
        assert!(text.contains("Used by work and home"), "{text}");
    }

    #[test]
    fn sibling_folders_collapse_into_their_parent() {
        let root = paths::home().join("x").join("scratch");
        let mut owners = BTreeMap::new();
        for n in ["a", "b", "c"] {
            owners.insert(root.join(n), vec!["w".to_string()]);
        }
        owners.insert(root.join("d"), vec!["p".to_string()]);
        let (out, guessed) = collapse_siblings(owners);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[&root], vec!["w".to_string()]);
        assert!(guessed.contains(&root));
    }
}
