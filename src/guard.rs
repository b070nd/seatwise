//! The air gap: which folders a seat may start in and touch, and which org it must be signed into.
//! Used by the launcher and by the two Claude Code hooks (`seat hook prompt|tool <seat>`).

use crate::config::{Config, Enforcement, Owner, Seat};
use crate::folders::{normalize, owner_of};
use crate::paths;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// `SEATWISE_FORCE=1` gets one start past the start checks (e.g. to run `/login` on a seat signed into the wrong
/// org). Honoured only from a terminal you type in: never inside an agent session and never in a hook (its input
/// is a pipe), so a session can't lift its own guard. The hooks of a forced session still check it.
pub fn forced() -> bool {
    force_honoured(
        std::env::var_os("SEATWISE_FORCE").is_some_and(|v| !v.is_empty()),
        crate::privacy::agent(),
        std::io::IsTerminal::is_terminal(&std::io::stdin()),
    )
}

/// Whether `SEATWISE_FORCE` is set but ignored (inside an agent, or without a terminal).
pub fn force_ignored() -> bool {
    std::env::var_os("SEATWISE_FORCE").is_some_and(|v| !v.is_empty()) && !forced()
}

fn force_honoured(set: bool, agent: bool, terminal: bool) -> bool {
    set && !agent && terminal
}

/// Variables that pick a seat or lift its guard. A shell command that sets or clears one would start the agent (or
/// seatwise) on another seat's login, another config, or without the checks.
pub const GUARD_VARS: [&str; 6] = [
    "SEATWISE_FORCE",
    "SEATWISE_SEAT",
    "SEATWISE_CONFIG",
    "SEATWISE_REVEAL",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
];

/// Why the guard says no: what is wrong and, separately, what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub message: String,
    pub resolution: Option<String>,
}

impl Refusal {
    fn new(message: String) -> Self {
        Refusal {
            message,
            resolution: None,
        }
    }

    fn resolve(mut self, resolution: String) -> Self {
        self.resolution = Some(resolution);
        self
    }
}

/// One line, for hooks and warnings: the message and then the resolution.
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)?;
        match &self.resolution {
            Some(r) => write!(f, " {r}"),
            None => Ok(()),
        }
    }
}

impl From<Refusal> for crate::error::UserError {
    fn from(r: Refusal) -> Self {
        let mut e = crate::error::UserError::new(r.message).code(crate::error::Code::Refused);
        e.resolution = r.resolution;
        e
    }
}

/// Starting (or prompting) in `dir` as `seat`: the folder must belong to the seat or be shared (`any`).
pub fn check_start(cfg: &Config, seat: &Seat, dir: &Path) -> Result<(), Refusal> {
    match owner_of(cfg, dir) {
        Some(Owner::Any) => Ok(()),
        Some(Owner::Seat(s)) if s == seat.name => Ok(()),
        Some(Owner::Seat(s)) => Err(Refusal::new(format!(
            "{} belongs to seat '{s}', not '{}'.",
            dir.display(),
            seat.name
        ))
        .resolve(format!("Use: seat run {s}"))),
        Some(Owner::Nobody) => Err(Refusal::new(format!(
            "{} is closed to every seat (owner = \"none\")",
            dir.display()
        ))),
        None => Err(
            Refusal::new(format!("{} is not assigned to a seat.", dir.display())).resolve(format!(
                "Add a [[folder]] entry to {} (seat link <seat> <folder>)",
                paths::config_file().display()
            )),
        ),
    }
}

/// Touching `path` from a `seat` session: blocked only when another seat (or `none`) owns it.
pub fn check_access(cfg: &Config, seat: &Seat, path: &Path) -> Result<(), Refusal> {
    match owner_of(cfg, path) {
        None | Some(Owner::Any) => Ok(()),
        Some(Owner::Seat(s)) if s == seat.name => Ok(()),
        Some(Owner::Seat(s)) => Err(Refusal::new(format!(
            "{} belongs to seat '{s}'; the '{}' seat can't access it.",
            path.display(),
            seat.name
        ))),
        Some(Owner::Nobody) => Err(Refusal::new(format!(
            "{} is closed to every seat.",
            path.display()
        ))),
    }
}

/// The seat must be signed into its pinned org and as its pinned email. Signed out (or unpinned) passes:
/// there is nothing to compare yet, and logging in must stay possible.
pub fn check_identity(seat: &Seat, org: Option<&str>, email: Option<&str>) -> Result<(), Refusal> {
    if let (Some(want), Some(got)) = (seat.org_uuid.as_deref(), org)
        && want != got
    {
        // No identifiers in the message: it can reach the model (hooks) and leave the machine.
        return Err(Refusal::new(format!(
            "seat '{}' is signed into another org than the one pinned for it.",
            seat.name
        ))
        .resolve(
            "Run /login and pick the pinned org (`seat list` shows it on your terminal; SEATWISE_FORCE=1 to start \
             anyway)."
                .into(),
        ));
    }
    if let (Some(want), Some(got)) = (seat.email.as_deref(), email)
        && !want.eq_ignore_ascii_case(got)
    {
        return Err(Refusal::new(format!(
            "seat '{}' is signed in with another account than the one pinned for it.",
            seat.name
        ))
        .resolve("Run /login with the pinned account (SEATWISE_FORCE=1 to start anyway).".into()));
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
    fn at(level: Enforcement, check: Result<(), Refusal>) -> Self {
        match (check, level) {
            (Ok(()), _) | (Err(_), Enforcement::Off) => Verdict::Allow,
            (Err(m), Enforcement::Warn) => Verdict::Warn(m.to_string()),
            (Err(m), Enforcement::Strict) => Verdict::Block(m.to_string()),
        }
    }
}

/// UserPromptSubmit: prompts in a folder or org that isn't this seat's.
pub fn hook_prompt(cfg: &Config, seat: &Seat, input: &Value) -> Verdict {
    // No SEATWISE_FORCE here: a session must not be able to lift its own guard.
    let level = cfg.enforcement_of(seat);
    if level == Enforcement::Off {
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
    // No SEATWISE_FORCE here: a session must not be able to lift its own guard.
    let level = cfg.enforcement_of(seat);
    if level == Enforcement::Off {
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
    let check = check_command(&input["tool_input"]["command"])
        .and_then(|()| paths.iter().try_for_each(|p| check_access(cfg, seat, p)));
    Verdict::at(level, check)
}

/// A shell command (a string, or a command vector as Codex sends it) must not set or clear a [`GUARD_VARS`]
/// variable. Only assignments that take effect count: words at the start of a command (`VAR=1 claude`), after
/// `env`, `export`, `declare`, `typeset`, `local`, `readonly`, `unset`, `setenv`, `set -x`, and PowerShell's
/// `$env:VAR`. Quoted words with spaces are read as commands too (`bash -c '...'`, `eval "..."`).
pub fn check_command(command: &Value) -> Result<(), Refusal> {
    let found = match command {
        Value::String(s) => command_sets(s, 0),
        Value::Array(a) => {
            let words: Vec<String> = a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            words_set(&words, 0)
        }
        _ => None,
    };
    match found {
        None => Ok(()),
        Some(var) => Err(Refusal::new(format!(
            "this command sets or clears {var}, which would get around the seat's guard."
        ))
        .resolve("Run it in your own terminal if you mean to.".into())),
    }
}

fn command_sets(cmd: &str, depth: usize) -> Option<&'static str> {
    simple_commands(cmd)
        .iter()
        .find_map(|w| words_set(w, depth))
}

/// The guard variable one simple command sets or clears, if any.
fn words_set(words: &[String], depth: usize) -> Option<&'static str> {
    let var_of = |w: &str| {
        let name = w.split_once('=').map_or(w, |(n, _)| n);
        GUARD_VARS.iter().copied().find(|v| *v == name)
    };
    let ps_env = |w: &str| {
        let lower = w.to_ascii_lowercase();
        ["$env:", "env:"]
            .iter()
            .find_map(|p| lower.starts_with(p).then(|| var_of(&w[p.len()..])))
            .flatten()
    };
    if let Some(v) = words.iter().find_map(|w| ps_env(w)) {
        return Some(v);
    }
    let mut i = 0;
    // Assignments and wrappers in front of the command: `VAR=1`, `env [-u VAR] [VAR=1]`, `sudo`, `exec`, ...
    while let Some(w) = words.get(i) {
        if w.contains('=') && !w.starts_with('-') {
            if let Some(v) = var_of(w) {
                return Some(v);
            }
            i += 1;
            continue;
        }
        let base = w.rsplit(['/', '\\']).next().unwrap_or(w);
        match base {
            "env" | "sudo" | "doas" | "exec" | "command" | "builtin" | "nohup" | "nice"
            | "time" => {
                i += 1;
                while let Some(o) = words.get(i).filter(|o| o.starts_with('-')) {
                    if base == "env" && matches!(o.as_str(), "-u" | "--unset") {
                        if let Some(v) = words.get(i + 1).and_then(|n| var_of(n)) {
                            return Some(v);
                        }
                        i += 1;
                    } else if let Some(v) = o.strip_prefix("--unset=").and_then(var_of) {
                        return Some(v);
                    }
                    i += 1;
                }
            }
            _ => break,
        }
    }
    let head = words.get(i)?;
    let rest = &words[i + 1..];
    let base = head.rsplit(['/', '\\']).next().unwrap_or(head);
    match base {
        "export" | "declare" | "typeset" | "local" | "readonly" | "unset" | "setenv" | "set" => {
            rest.iter().find_map(|w| var_of(w))
        }
        "eval" if depth < 3 => command_sets(&rest.join(" "), depth + 1),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "pwsh" | "powershell" if depth < 3 => {
            rest.iter()
                .position(|o| {
                    o.eq_ignore_ascii_case("-command")
                        || (o.starts_with('-') && !o.starts_with("--") && o.ends_with('c'))
                })
                .and_then(|at| rest.get(at + 1))
                .and_then(|script| command_sets(script, depth + 1))
        }
        _ => None,
    }
}

/// A shell command split into simple commands (at `; | & ( )`, newlines and backticks), each as its words.
fn simple_commands(cmd: &str) -> Vec<Vec<String>> {
    let mut out = vec![Vec::new()];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    let end_word = |cur: &mut String, out: &mut Vec<Vec<String>>| {
        if !cur.is_empty() {
            out.last_mut().unwrap().push(std::mem::take(cur));
        }
    };
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None => match c {
                '\'' | '"' => quote = Some(c),
                '\\' => {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                    }
                }
                c if ";|&()`\n".contains(c) => {
                    end_word(&mut cur, &mut out);
                    if !out.last().unwrap().is_empty() {
                        out.push(Vec::new());
                    }
                }
                c if c.is_whitespace() => end_word(&mut cur, &mut out),
                c => cur.push(c),
            },
        }
    }
    end_word(&mut cur, &mut out);
    out.retain(|w| !w.is_empty());
    out
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
        let refused = check_start(&cfg, b, &root.join("a/x")).unwrap_err();
        assert!(refused.message.contains("seat 'a'"));
        assert_eq!(refused.resolution.as_deref(), Some("Use: seat run a"));
        assert!(
            refused.to_string().ends_with("Use: seat run a"),
            "hooks get one line"
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
    fn force_needs_a_terminal_outside_agents() {
        assert!(force_honoured(true, false, true));
        assert!(!force_honoured(true, true, true), "inside an agent");
        assert!(
            !force_honoured(true, false, false),
            "no terminal (an agent's shell, a hook)"
        );
        assert!(!force_honoured(false, false, true));
    }

    #[test]
    fn commands_that_lift_the_guard() {
        let sets = |c: &str| check_command(&json!(c)).is_err();
        assert!(sets("A=1 SEATWISE_FORCE=1 claude"));
        assert!(sets("sudo -E env -i CODEX_HOME=/x codex"));
        assert!(sets("env --unset=SEATWISE_SEAT seat run auto"));
        assert!(sets("(cd x; export SEATWISE_REVEAL=1)"));
        assert!(sets("echo $(SEATWISE_SEAT=b seat list)"));
        assert!(sets("/bin/zsh -ic \"unset CLAUDE_CONFIG_DIR\""));
        assert!(sets("eval 'SEATWISE_CONFIG=/x seat run'"));
        assert!(check_command(&json!(["bash", "-lc", "SEATWISE_SEAT=b claude"])).is_err());
        assert!(
            !sets("echo SEATWISE_FORCE=1"),
            "an argument, not an assignment"
        );
        assert!(!sets("printenv CLAUDE_CONFIG_DIR"));
        assert!(!sets("set -euo pipefail"));
        assert!(!sets("FOO=1 make"));
        assert!(check_command(&json!(null)).is_ok());
    }

    #[test]
    fn enforcement_levels() {
        let err = || Err(Refusal::new("x".to_string()));
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
