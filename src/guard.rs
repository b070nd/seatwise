//! The air gap: which folders a seat may start in and touch, and which org it must be signed into.
//! Used by the launcher and by the two Claude Code hooks (`seat hook prompt|tool <seat>`).

use crate::config::{Config, Enforcement, Owner, Seat};
use crate::folders::{normalize, owner_of};
use crate::paths;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// `SEATWISE_FORCE=1` skips every check (e.g. to run `/login` on a seat signed into the wrong org).
pub fn forced() -> bool {
    std::env::var_os("SEATWISE_FORCE").is_some_and(|v| !v.is_empty())
}

/// Starting (or prompting) in `dir` as `seat`: the folder must belong to the seat or be shared (`any`).
pub fn check_start(cfg: &Config, seat: &Seat, dir: &Path) -> Result<(), String> {
    match owner_of(cfg, dir) {
        Some(Owner::Any) => Ok(()),
        Some(Owner::Seat(s)) if s == seat.name => Ok(()),
        Some(Owner::Seat(s)) => Err(format!(
            "{} belongs to seat '{s}', not '{}'. Use: seat run {s}",
            dir.display(),
            seat.name
        )),
        Some(Owner::Nobody) => Err(format!(
            "{} is closed to every seat (owner = \"none\")",
            dir.display()
        )),
        None => Err(format!(
            "{} is not assigned to a seat. Add a [[folder]] entry to {}",
            dir.display(),
            paths::config_file().display()
        )),
    }
}

/// Touching `path` from a `seat` session: blocked only when another seat (or `none`) owns it.
pub fn check_access(cfg: &Config, seat: &Seat, path: &Path) -> Result<(), String> {
    match owner_of(cfg, path) {
        None | Some(Owner::Any) => Ok(()),
        Some(Owner::Seat(s)) if s == seat.name => Ok(()),
        Some(Owner::Seat(s)) => Err(format!(
            "{} belongs to seat '{s}'; the '{}' seat can't access it.",
            path.display(),
            seat.name
        )),
        Some(Owner::Nobody) => Err(format!("{} is closed to every seat.", path.display())),
    }
}

/// The seat must be signed into its pinned org and as its pinned email. Signed out (or unpinned) passes:
/// there is nothing to compare yet, and logging in must stay possible.
pub fn check_identity(seat: &Seat, org: Option<&str>, email: Option<&str>) -> Result<(), String> {
    if let (Some(want), Some(got)) = (seat.org_uuid.as_deref(), org)
        && want != got
    {
        return Err(format!(
            "seat '{}' is signed into the wrong org ({got}). Run /login and pick {} (SEATWISE_FORCE=1 to start anyway).",
            seat.name,
            seat.org_name.as_deref().unwrap_or(want)
        ));
    }
    if let (Some(want), Some(got)) = (seat.email.as_deref(), email)
        && !want.eq_ignore_ascii_case(got)
    {
        return Err(format!(
            "seat '{}' is signed in as {got}, not {want}. Run /login with the right account (SEATWISE_FORCE=1 to start anyway).",
            seat.name
        ));
    }
    Ok(())
}

/// What a hook tells Claude Code.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Let it through, but tell the user (the tool hook asks for confirmation).
    Warn(String),
    Block(String),
}

impl Verdict {
    fn at(level: Enforcement, check: Result<(), String>) -> Self {
        match (check, level) {
            (Ok(()), _) | (Err(_), Enforcement::Off) => Verdict::Allow,
            (Err(m), Enforcement::Warn) => Verdict::Warn(m),
            (Err(m), Enforcement::Strict) => Verdict::Block(m),
        }
    }
}

/// UserPromptSubmit: prompts in a folder or org that isn't this seat's.
pub fn hook_prompt(cfg: &Config, seat: &Seat, input: &Value) -> Verdict {
    let level = cfg.enforcement_of(seat);
    if forced() || level == Enforcement::Off {
        return Verdict::Allow;
    }
    let dir = std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .or_else(|| input["cwd"].as_str().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(paths::home);
    let start = check_start(cfg, seat, &normalize(&dir, &paths::home()));
    // The desktop app exports the org it is signed into; the CLI keeps it in the state file.
    let account = crate::claude::read_account(seat).unwrap_or_default();
    let org = std::env::var("CLAUDE_CODE_ORGANIZATION_UUID")
        .ok()
        .filter(|o| !o.is_empty())
        .or(account.org_uuid);
    Verdict::at(
        level,
        start.and_then(|()| check_identity(seat, org.as_deref(), account.email.as_deref())),
    )
}

/// PreToolUse: file tools and shell commands that reach into another seat's folders.
pub fn hook_tool(cfg: &Config, seat: &Seat, input: &Value) -> Verdict {
    let level = cfg.enforcement_of(seat);
    if forced() || level == Enforcement::Off {
        return Verdict::Allow;
    }
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(paths::home);
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let mut paths = tool_paths(tool, &input["tool_input"], &cwd);
    if seat.agent == crate::config::Agent::Codex {
        // Codex's tools (shell, apply_patch, ...) aren't Claude Code's: scan everything in the input.
        paths.extend(scan_any(&input["tool_input"], &cwd));
    }
    let check = paths.iter().try_for_each(|p| check_access(cfg, seat, p));
    Verdict::at(level, check)
}

/// Every path a tool call would touch, as far as can be told from its input.
/// Shell commands are scanned for absolute and home-relative paths only (best effort, see docs/security-model.md).
pub fn tool_paths(tool: &str, input: &Value, cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(s) = input[key].as_str() {
            out.push(normalize(&shell_path(s), cwd));
        }
    }
    if tool == "Glob"
        && let Some(pattern) = input["pattern"].as_str()
    {
        let fixed = &pattern[..pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len())];
        out.push(normalize(&shell_path(fixed), cwd));
    }
    if let Some(cmd) = input["command"].as_str() {
        for t in tokens(cmd) {
            let p = shell_path(&t);
            if p.has_root() {
                out.push(normalize(&p, cwd));
            }
        }
    }
    out
}

/// Paths anywhere in a tool input: absolute and home-relative words in every string (and array of strings, like
/// a command vector), and the file lines of a patch (`*** Update File: src/x.rs`), relative ones included.
pub fn scan_any(input: &Value, cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![input];
    while let Some(v) = stack.pop() {
        match v {
            Value::String(s) => {
                for line in s.lines() {
                    for head in [
                        "*** Add File: ",
                        "*** Update File: ",
                        "*** Delete File: ",
                        "*** Move to: ",
                    ] {
                        if let Some(p) = line.trim().strip_prefix(head) {
                            out.push(normalize(&shell_path(p.trim()), cwd));
                        }
                    }
                }
                for t in tokens(s) {
                    let p = shell_path(&t);
                    if p.has_root() {
                        out.push(normalize(&p, cwd));
                    }
                }
            }
            Value::Array(a) => stack.extend(a),
            Value::Object(o) => stack.extend(o.values()),
            _ => {}
        }
    }
    out
}

/// Expand home references; on Windows also accept Git Bash paths (`/c/Users/...`).
fn shell_path(s: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let b = s.as_bytes();
        if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
            return PathBuf::from(format!("{}:\\{}", b[1] as char, &s[3..]));
        }
    }
    paths::expand(s)
}

/// Split a shell command into words: quotes and `\ ` respected, split on whitespace and `; | & ( ) < > \``.
/// For `--flag=value` words the value is returned too.
pub fn tokens(cmd: &str) -> Vec<String> {
    fn flush(cur: &mut String, out: &mut Vec<String>) {
        if cur.is_empty() {
            return;
        }
        if let Some((_, value)) = cur.split_once('=')
            && !value.is_empty()
        {
            out.push(value.to_string());
        }
        out.push(std::mem::take(cur));
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None => match c {
                '\'' | '"' => quote = Some(c),
                '\\' if chars.peek().is_some_and(|n| n.is_whitespace()) => {
                    cur.push(chars.next().unwrap())
                }
                c if c.is_whitespace() || ";|&()<>`".contains(c) => flush(&mut cur, &mut out),
                c => cur.push(c),
            },
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Folder;
    use serde_json::json;

    #[test]
    fn tokenizes_like_a_shell() {
        assert_eq!(
            tokens(r#"cat "/a b/c" '/d e'|grep x; ls /f\ g --out=/h"#),
            vec![
                "cat", "/a b/c", "/d e", "grep", "x", "ls", "/f g", "/h", "--out=/h"
            ]
        );
    }

    #[test]
    fn finds_paths_in_tool_input() {
        // First components longer than one letter: on Windows `/x/...` reads as the Git Bash drive X:.
        let cwd = Path::new("/work");
        let paths = |tool: &str, input: Value| tool_paths(tool, &input, cwd);
        let norm = |p: &str| normalize(Path::new(p), cwd);
        assert_eq!(
            paths("Read", json!({"file_path": "/data/y"})),
            vec![norm("/data/y")]
        );
        assert_eq!(
            paths("Glob", json!({"pattern": "/data/**/*.rs"})),
            vec![norm("/data")]
        );
        assert_eq!(
            paths("Grep", json!({"pattern": "/not/a/path"})),
            Vec::<PathBuf>::new(),
            "Grep pattern is a regex"
        );
        let home = paths::home();
        let found = paths("Bash", json!({"command": "cd ~/p && cat /etc/hosts rel/x"}));
        assert!(
            found
                .iter()
                .any(|p| p.starts_with(normalize(&home, cwd).join("p"))),
            "{found:?}"
        );
        assert!(found.iter().any(|p| p.ends_with("hosts")), "{found:?}");
        assert!(
            !found.iter().any(|p| p.ends_with("x")),
            "relative words are not scanned"
        );
    }

    #[test]
    fn scans_codex_inputs() {
        let cwd = Path::new("/work");
        let found = scan_any(
            &json!({
                "command": ["bash", "-lc", "cat /data/secret"],
                "patch": "*** Begin Patch\n*** Update File: ../other/x.rs\n@@\n*** End Patch"
            }),
            cwd,
        );
        assert!(found.iter().any(|p| p.ends_with("secret")), "{found:?}");
        assert!(
            found
                .iter()
                .any(|p| p.ends_with("other/x.rs") && !p.starts_with("/work")),
            "relative patch paths resolve against cwd: {found:?}"
        );
    }

    #[test]
    fn decisions() {
        let t = tempfile::tempdir().unwrap();
        let root = normalize(t.path(), Path::new("/"));
        let folder = |p: &str, o: &str| Folder {
            path: root.join(p).to_string_lossy().into(),
            owner: o.into(),
        };
        let seat = |n: &str| Seat {
            name: n.into(),
            config_dir: format!("~/.c-{n}"),
            ..Seat::default()
        };
        let cfg = Config {
            enforcement: Enforcement::Strict,
            seats: vec![seat("a"), seat("b")],
            folders: vec![
                folder("", "none"),
                folder("a", "a"),
                folder("a/b", "b"),
                folder("start", "any"),
            ],
            ..Config::default()
        };
        let (a, b) = (&cfg.seats[0], &cfg.seats[1]);
        assert!(check_start(&cfg, a, &root.join("a/x")).is_ok());
        assert!(
            check_start(&cfg, b, &root.join("a/x"))
                .unwrap_err()
                .contains("seat 'a'")
        );
        assert!(check_start(&cfg, b, &root.join("start")).is_ok());
        assert!(check_start(&cfg, a, &root.join("other")).is_err(), "none");
        assert!(check_start(&cfg, a, Path::new("/unassigned")).is_err());
        assert!(
            check_access(&cfg, a, &root.join("a/b/f")).is_err(),
            "nested folder of another seat"
        );
        assert!(
            check_access(&cfg, a, Path::new("/usr/lib")).is_ok(),
            "unassigned is readable"
        );

        let mut a2 = a.clone();
        a2.org_uuid = Some("org-a".into());
        a2.email = Some("Me@Example.com".into());
        assert!(check_identity(&a2, Some("org-a"), Some("me@example.com")).is_ok());
        assert!(check_identity(&a2, Some("org-b"), None).is_err());
        assert!(check_identity(&a2, Some("org-a"), Some("other@example.com")).is_err());
        assert!(
            check_identity(&a2, None, None).is_ok(),
            "logged out: let them log in"
        );
    }

    #[test]
    fn enforcement_levels() {
        let err = || Err("x".to_string());
        assert_eq!(Verdict::at(Enforcement::Off, err()), Verdict::Allow);
        assert_eq!(
            Verdict::at(Enforcement::Warn, err()),
            Verdict::Warn("x".into())
        );
        assert_eq!(
            Verdict::at(Enforcement::Strict, err()),
            Verdict::Block("x".into())
        );
        assert_eq!(Verdict::at(Enforcement::Strict, Ok(())), Verdict::Allow);
    }
}
