//! End-to-end: the real `seat` binary against a temp home, with a fake `claude` that prints what it got.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
    fake_claude: PathBuf,
}

/// The air-gap setup: strict enforcement, snapshots on.
fn setup() -> Env {
    setup_with(
        r#"enforcement = "strict"
        track = ["settings.json", "CLAUDE.md"]"#,
    )
}

/// `top` goes before the seats: global options.
fn setup_with(top: &str) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = dunce::canonicalize(tmp.path()).unwrap();
    for dir in [
        "proj-a/nested",
        "proj-b",
        "start",
        "closed",
        "elsewhere",
        ".seat-a",
        ".seat-b",
    ] {
        fs::create_dir_all(home.join(dir)).unwrap();
    }
    let account = |org: &str| {
        format!(
            r#"{{"oauthAccount":{{"organizationUuid":"{org}","organizationName":"Org {org}"}}}}"#
        )
    };
    fs::write(home.join(".seat-a/.claude.json"), account("org-a")).unwrap();
    fs::write(home.join(".seat-b/.claude.json"), account("org-x")).unwrap(); // wrong org on purpose
    sign_in(&home.join(".seat-a"));
    sign_in(&home.join(".seat-b"));
    let config = home.join("config.toml");
    fs::write(
        &config,
        top.to_string()
            + r#"
        [[seat]]
        name = "a"
        aliases = ["alpha"]
        config_dir = "~/.seat-a"
        org_uuid = "org-a"
        [[seat]]
        name = "b"
        config_dir = "~/.seat-b"
        org_uuid = "org-b"
        link = "copy"
        [[folder]]
        path = "~/proj-a"
        owner = "a"
        [[folder]]
        path = "~/proj-a/nested"
        owner = "b"
        [[folder]]
        path = "~/proj-b"
        owner = "b"
        [[folder]]
        path = "~/start"
        owner = "any"
        [[folder]]
        path = "~/closed"
        owner = "none"
        "#,
    )
    .unwrap();
    // The seat binary acts as a fake claude when named `fake-claude` (prints what it was given). Same on every OS.
    let fake_claude = home.join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_seat"), &fake_claude).unwrap();
    Env {
        _tmp: tmp,
        home,
        config,
        fake_claude,
    }
}

impl Env {
    fn seat(&self, dir: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_seat"));
        cmd.args(args)
            .current_dir(self.home.join(dir))
            .env("SEATWISE_HOME", &self.home)
            .env("SEATWISE_CONFIG", &self.config)
            .env("SEATWISE_DATA", self.home.join("data"))
            .env("SEATWISE_CLAUDE", &self.fake_claude)
            .env("ANTHROPIC_API_KEY", "leaked")
            // Tests see real values; redaction has its own test.
            .env("SEATWISE_REVEAL", "1")
            // Never look at this machine's keychain: logins come from the fixtures' .credentials.json.
            .env("SEATWISE_NO_KEYCHAIN", "1")
            .env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT")
            .env_remove("CODEX_SANDBOX")
            .env_remove("CODEX_SANDBOX_NETWORK_DISABLED")
            .env_remove("SEATWISE_PRIVATE")
            .env_remove("SEATWISE_FORCE")
            .env_remove("SEATWISE_SEAT")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_CODE_ORGANIZATION_UUID");
        cmd
    }

    fn hook(&self, kind: &str, seat: &str, input: serde_json::Value) -> Output {
        use std::io::Write;
        let mut child = self
            .seat("", &["hook", kind, seat])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

/// A fake Claude Code login for a config folder (`.claude.json` alone only records the account).
fn sign_in(config_dir: &Path) {
    fs::write(
        config_dir.join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fake","expiresAt":99999999999999}}"#,
    )
    .unwrap();
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn p(path: &Path) -> String {
    path.display().to_string()
}

#[test]
fn auto_picks_the_folder_owner_and_isolates_the_environment() {
    let e = setup();
    let out = e
        .seat("proj-a", &["run", "auto", "--continue"])
        .output()
        .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains(&format!("CFG={}", p(&e.home.join(".seat-a")))),
        "{t}"
    );
    assert!(t.contains("SEAT=a"), "{t}");
    assert!(t.contains("KEY=none"), "API key must not leak: {t}");
    assert!(t.contains("ARGS=--continue"), "{t}");
}

#[test]
fn wrong_folder_unassigned_closed_and_shared_auto_are_refused() {
    let e = setup();
    let t = text(&e.seat("proj-b", &["run", "alpha"]).output().unwrap());
    assert!(
        t.contains("belongs to seat 'b'") && !t.contains("CFG="),
        "{t}"
    );
    let t = text(&e.seat("elsewhere", &["run", "a"]).output().unwrap());
    assert!(t.contains("not assigned") && !t.contains("CFG="), "{t}");
    let t = text(&e.seat("closed", &["run", "a"]).output().unwrap());
    assert!(
        t.contains("closed to every seat") && !t.contains("CFG="),
        "{t}"
    );
    let t = text(&e.seat("start", &["run", "auto"]).output().unwrap());
    assert!(
        t.contains("shared start folder") && !t.contains("CFG="),
        "{t}"
    );
}

#[test]
fn a_worktree_outside_every_folder_runs_on_its_main_checkouts_seat() {
    let e = setup();
    let gitdir = e.home.join("proj-a/.git/worktrees/t1");
    fs::create_dir_all(&gitdir).unwrap();
    let wt = e.home.join("elsewhere/thread/proj-a");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
    let t = text(
        &e.seat("elsewhere/thread/proj-a", &["run", "auto"])
            .output()
            .unwrap(),
    );
    assert!(t.contains("SEAT=a"), "{t}");
    // The other seat is still refused there.
    let t = text(
        &e.seat("elsewhere/thread/proj-a", &["run", "b"])
            .output()
            .unwrap(),
    );
    assert!(
        t.contains("belongs to seat 'a'") && !t.contains("CFG="),
        "{t}"
    );
}

#[test]
fn share_adopt_copies_tool_skills_into_the_seats_and_import_reads_a_projects_file() {
    let e = setup_with("");
    let skill = e.home.join(".claude/skills/tool-x");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "tool").unwrap();
    let t = ok(&e.seat("", &["share", "--adopt", "a"]).output().unwrap());
    assert!(t.contains("seat 'a': shares skills/tool-x"), "{t}");
    assert_eq!(
        fs::read_to_string(e.home.join(".seat-a/skills/tool-x/SKILL.md")).unwrap(),
        "tool"
    );
    assert!(
        !e.home.join(".seat-b/skills/tool-x").exists(),
        "only the named seat"
    );
    assert!(read(&e.config).contains("skills/tool-x"));

    fs::write(
        e.home.join("projects"),
        format!(
            "# old launcher\nb {}\nnone {}\n",
            p(&e.home.join("elsewhere")),
            p(&e.home.join("closed"))
        ),
    )
    .unwrap();
    ok(&e
        .seat("", &["import", "projects", &p(&e.home.join("projects"))])
        .output()
        .unwrap());
    assert!(
        ok(&e
            .seat("", &["owner", &p(&e.home.join("elsewhere"))])
            .output()
            .unwrap())
        .contains("b")
    );
}

#[test]
fn doctor_flags_nested_seat_folders_and_an_unguarded_codex_home() {
    let e = setup_with("");
    ok(&e
        .seat("", &["link", "b", &p(&e.home.join("proj-a/nested"))])
        .output()
        .unwrap());
    fs::create_dir_all(e.home.join(".codex")).unwrap();
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(
        t.contains("belongs to seat 'a' but holds folders of seat 'b'"),
        "{t}"
    );
    assert!(
        t.contains("~/.codex exists but no Codex seat uses it"),
        "{t}"
    );
}

#[test]
fn unknown_names_say_what_was_probably_meant() {
    let e = setup();
    let t = text(&e.seat("proj-a", &["run", "alpah"]).output().unwrap());
    assert!(
        t.contains("unknown seat 'alpah' (maybe you meant 'alpha'; known: a, alpha, b"),
        "{t}"
    );
    let t = text(&e.seat("", &["show", "c"]).output().unwrap());
    assert!(t.contains("unknown seat 'c' (known: "), "{t}");
    let t = text(&e.seat("", &["preset", "show", "buidl"]).output().unwrap());
    assert!(
        t.contains("unknown preset 'buidl' (maybe you meant 'build'"),
        "{t}"
    );
    let t = text(&e.seat("", &["link", "bb", "elsewhere"]).output().unwrap());
    assert!(
        t.contains("maybe you meant 'b'") && t.contains("or use \"any\""),
        "{t}"
    );
}

#[test]
fn failures_exit_with_a_code_that_says_why_and_print_a_resolution() {
    let e = setup();
    // Refused by the guard: 3, with the way out on its own line.
    let out = e.seat("proj-b", &["run", "alpha"]).output().unwrap();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(3), "{t}");
    assert!(
        t.contains("belongs to seat 'b', not 'a'.")
            && t.contains("\n  resolution: Use: seat run b"),
        "{t}"
    );
    // No such seat / preset: 4.
    for args in [&["show", "nobody"][..], &["preset", "show", "nothing"]] {
        let out = e.seat("", args).output().unwrap();
        assert_eq!(out.status.code(), Some(4), "{}", text(&out));
    }
    // Usage error: 2 (clap). Anything else: 1.
    assert_eq!(
        e.seat("", &["--bogus"]).output().unwrap().status.code(),
        Some(2)
    );
    let out = e
        .seat("", &["link", "--from", "/no/such/file"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
}

#[test]
fn blocks_are_logged_and_failures_name_their_log() {
    let e = setup();
    // A failing run leaves a run log and points at it.
    let out = e.seat("proj-b", &["run", "alpha"]).output().unwrap();
    let t = text(&out);
    let line = t.lines().find(|l| l.starts_with("  log: ")).expect(&t);
    let logs = e.home.join("data/logs");
    let files: Vec<_> = fs::read_dir(&logs)
        .unwrap()
        .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(line.contains(&files[0]), "{line} vs {files:?}");
    let log = read(&logs.join(&files[0]));
    assert!(
        log.contains("run seat=a") && log.contains("guard (Strict): ") && log.contains("error: "),
        "{log}"
    );
    // A blocked prompt is recorded in hooks.log, and `seat log` shows it.
    let blocked = e.hook(
        "prompt",
        "b",
        serde_json::json!({ "cwd": p(&e.home.join("proj-a")), "prompt": "hi" }),
    );
    assert_eq!(blocked.status.code(), Some(2));
    let shown = ok(&e.seat("", &["log"]).output().unwrap());
    assert!(
        shown.contains("block prompt seat=b: ") && shown.contains("run logs: "),
        "{shown}"
    );
    // Allowed calls are not logged.
    let allowed = e.hook(
        "prompt",
        "a",
        serde_json::json!({ "cwd": p(&e.home.join("proj-a")), "prompt": "hi" }),
    );
    assert_eq!(allowed.status.code(), Some(0));
    assert_eq!(read(&logs.join("hooks.log")).lines().count(), 1);
}

#[test]
fn version_and_help_probes_run_in_folders_no_seat_owns() {
    let e = setup();
    let t = text(
        &e.seat("elsewhere", &["run", "auto", "--version"])
            .output()
            .unwrap(),
    );
    assert!(t.contains("ARGS=--version") && t.contains("SEAT="), "{t}");
    // A real session there is still refused.
    let t = text(&e.seat("elsewhere", &["run", "auto"]).output().unwrap());
    assert!(t.contains("not assigned") && !t.contains("CFG="), "{t}");
}

#[test]
fn shared_start_folder_grants_only_the_seats_own_folders() {
    let e = setup();
    let out = e.seat("start", &["run", "a", "-p", "hi"]).output().unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("--settings") && t.contains("additionalDirectories"),
        "{t}"
    );
    assert!(t.contains("proj-a") && !t.contains("proj-b"), "{t}");
}

#[test]
fn wrong_org_is_refused_and_force_needs_a_terminal() {
    let e = setup();
    let t = text(&e.seat("proj-b", &["run", "auto"]).output().unwrap());
    assert!(t.contains("another org") && !t.contains("CFG="), "{t}");
    // No terminal here (as for an agent's shell): SEATWISE_FORCE counts for nothing.
    for agent in [false, true] {
        let mut cmd = e.seat("proj-b", &["run", "auto"]);
        cmd.env("SEATWISE_FORCE", "1");
        if agent {
            cmd.env("CLAUDECODE", "1");
        }
        let out = cmd.output().unwrap();
        let t = text(&out);
        assert_eq!(out.status.code(), Some(3), "{t}");
        assert!(
            t.contains("SEATWISE_FORCE is ignored") && !t.contains("SEAT=b"),
            "{t}"
        );
    }
}

#[test]
fn tool_hook_blocks_commands_that_lift_the_guard() {
    let e = setup();
    let bash = |cmd: &str| serde_json::json!({ "tool_name": "Bash", "tool_input": { "command": cmd }, "cwd": p(&e.home.join("proj-a")) });
    for cmd in [
        "SEATWISE_FORCE=1 claude -p hi",
        "cd /tmp && env -u SEATWISE_SEAT seat run auto",
        "export CLAUDE_CONFIG_DIR=/elsewhere; claude",
        "unset SEATWISE_CONFIG",
        "bash -lc 'SEATWISE_SEAT=b claude'",
        "$env:CLAUDE_CONFIG_DIR = 'x'",
    ] {
        let out = e.hook("tool", "a", bash(cmd));
        assert_eq!(out.status.code(), Some(2), "{cmd}: {}", text(&out));
    }
    for cmd in [
        "grep -rn SEATWISE_FORCE= src",
        "echo $CLAUDE_CONFIG_DIR",
        "git commit -m 'SEATWISE_FORCE=1 needs a terminal now'",
    ] {
        let out = e.hook("tool", "a", bash(cmd));
        assert_eq!(out.status.code(), Some(0), "{cmd}: {}", text(&out));
    }
    // A session started with SEATWISE_FORCE in its environment is still checked.
    let read = serde_json::json!({ "tool_name": "Read", "tool_input": { "file_path": p(&e.home.join("proj-b/x")) }, "cwd": p(&e.home.join("proj-a")) });
    let mut child = e.seat("", &["hook", "tool", "a"]);
    child.env("SEATWISE_FORCE", "1");
    let out = {
        use std::io::Write;
        let mut c = child
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin
            .take()
            .unwrap()
            .write_all(read.to_string().as_bytes())
            .unwrap();
        c.wait_with_output().unwrap()
    };
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}

#[test]
fn strict_seat_puts_its_hooks_back_or_does_not_start() {
    let e = setup();
    let settings = e.home.join(".seat-a/settings.json");
    let _ = fs::remove_file(&settings);
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(
        t.contains("guard hooks were missing") && t.contains("SEAT=a"),
        "{t}"
    );
    let v = fs::read_to_string(&settings).unwrap();
    assert!(v.contains("hook tool a --contract"), "{v}");
    // Hooks can't be written (settings.json isn't JSON): a strict seat refuses to start.
    fs::write(&settings, "{ not json").unwrap();
    let out = e.seat("proj-a", &["run", "a"]).output().unwrap();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(3), "{t}");
    assert!(
        t.contains("couldn't be put back") && !t.contains("SEAT=a"),
        "{t}"
    );
    assert!(t.contains("resolution: Fix"), "{t}");
}

#[test]
fn identities_and_session_wiring_do_not_reach_the_agent() {
    let e = setup();
    let run = |inside: bool| {
        let mut cmd = e.seat("proj-a", &["run", "a"]);
        cmd.env("CLAUDE_CODE_ACCOUNT_UUID", "acct")
            .env("CLAUDE_CODE_USER_EMAIL", "me@example.com")
            .env("CLAUDE_CODE_WARN_DISK_FREE_GIB", "7")
            .env("ANTHROPIC_BASE_URL", "http://localhost:9");
        if inside {
            cmd.env("CLAUDECODE", "1");
        }
        text(&cmd.output().unwrap())
    };
    let t = run(false);
    assert!(
        t.contains("DISKWARN=7") && t.contains("BASE=http://localhost:9"),
        "own shell settings pass: {t}"
    );
    assert!(t.contains("IDS=none"), "{t}");
    let t = run(true);
    assert!(
        t.contains("DISKWARN=\n") || t.contains("DISKWARN=\r"),
        "a session's wiring doesn't: {t}"
    );
    assert!(t.contains("BASE=\n") || t.contains("BASE=\r"), "{t}");
    assert!(t.contains("IDS=none"), "{t}");
}

#[test]
fn tool_hook_blocks_other_seats_paths() {
    let e = setup();
    let input = |tool: &str, ti: serde_json::Value| serde_json::json!({ "tool_name": tool, "tool_input": ti, "cwd": p(&e.home.join("proj-a")) });
    let code = |o: Output| o.status.code();
    let read = |path: PathBuf| input("Read", serde_json::json!({ "file_path": p(&path) }));
    assert_eq!(
        code(e.hook("tool", "a", read(e.home.join("proj-a/x.rs")))),
        Some(0)
    );
    assert_eq!(
        code(e.hook("tool", "a", read(e.home.join("proj-b/x.rs")))),
        Some(2)
    );
    assert_eq!(
        code(e.hook("tool", "a", read(e.home.join("proj-a/nested/x")))),
        Some(2),
        "nested folder of seat b"
    );
    assert_eq!(
        code(e.hook("tool", "a", read(e.home.join("closed/x")))),
        Some(2)
    );
    assert_eq!(
        code(e.hook(
            "tool",
            "a",
            input("Read", serde_json::json!({ "file_path": "../proj-b/x" }))
        )),
        Some(2),
        "relative"
    );
    let bash = |cmd: &str| input("Bash", serde_json::json!({ "command": cmd }));
    assert_eq!(
        code(e.hook("tool", "a", bash("cat ~/proj-b/secret | wc -l"))),
        Some(2)
    );
    assert_eq!(
        code(e.hook("tool", "a", bash("cargo test && ls ~/proj-a"))),
        Some(0)
    );
    let blocked = e.hook("tool", "a", read(e.home.join("proj-b/x.rs")));
    assert!(
        text(&blocked).contains("belongs to seat 'b'"),
        "{}",
        text(&blocked)
    );
}

#[test]
fn prompt_hook_checks_folder_and_org_and_fails_closed() {
    let e = setup();
    let ask = |seat: &str, dir: &str| e.hook_with_dir("prompt", seat, dir);
    assert_eq!(ask("a", "proj-a").status.code(), Some(0));
    assert_eq!(ask("a", "proj-b").status.code(), Some(2));
    assert_eq!(
        ask("b", "proj-b").status.code(),
        Some(2),
        "seat b is on the wrong org"
    );
    assert_eq!(
        ask("ghost", "proj-a").status.code(),
        Some(2),
        "unknown seat fails closed"
    );
}

impl Env {
    fn hook_with_dir(&self, kind: &str, seat: &str, dir: &str) -> Output {
        use std::io::Write;
        let mut child = self
            .seat("", &["hook", kind, seat])
            .env("CLAUDE_PROJECT_DIR", self.home.join(dir))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"{}").unwrap();
        child.wait_with_output().unwrap()
    }
}

#[test]
fn install_hooks_is_idempotent_and_sync_tracks_changes() {
    let e = setup();
    let settings = e.home.join(".seat-a/settings.json");
    fs::write(&settings, r#"{"model":"keep-me"}"#).unwrap();
    for _ in 0..2 {
        let out = e.seat("", &["install-hooks", "a"]).output().unwrap();
        assert!(out.status.success(), "{}", text(&out));
    }
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(v["model"], "keep-me");
    assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(v["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);

    // Our own change is not reported as drift; an outside change is, and can be undone.
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(!t.contains("changed since"), "{t}");
    fs::write(&settings, r#"{"reset":true}"#).unwrap();
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("changed since the last start"), "{t}");
    let out = e.seat("", &["sync", "restore", "a"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert!(fs::read_to_string(&settings).unwrap().contains("keep-me"));

    // A deleted file comes back.
    fs::remove_file(&settings).unwrap();
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("restored"), "{t}");
    assert!(settings.exists());
}

#[test]
fn enforcement_is_off_by_default() {
    let e = setup_with("");
    // Another seat's folder: started anyway; folders only pick the seat.
    let out = e.seat("proj-b", &["run", "a"]).output().unwrap();
    assert!(text(&out).contains("SEAT=a"), "{}", text(&out));
    // A wrong org is still worth a warning, but doesn't stop the start.
    let t = text(&e.seat("proj-b", &["run", "auto"]).output().unwrap());
    assert!(
        t.contains("warning") && t.contains("another org") && t.contains("SEAT=b"),
        "{t}"
    );
    // Unassigned folder, no default seat, no terminal: asks for a seat.
    let t = text(&e.seat("elsewhere", &["run", "auto"]).output().unwrap());
    assert!(t.contains("pick a seat") && !t.contains("CFG="), "{t}");
    // Hooks allow everything, and install-hooks doesn't add any.
    let read = serde_json::json!({ "tool_name": "Read", "tool_input": { "file_path": p(&e.home.join("proj-b/x")) }, "cwd": p(&e.home.join("proj-a")) });
    assert_eq!(e.hook("tool", "a", read).status.code(), Some(0));
    let out = e.seat("", &["install-hooks"]).output().unwrap();
    assert!(text(&out).contains("enforcement is off"), "{}", text(&out));
    assert!(
        !e.home.join(".seat-a/settings.json").exists(),
        "no file created"
    );
    // Snapshots are off too.
    fs::write(e.home.join(".seat-a/settings.json"), "{}").unwrap();
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    fs::write(e.home.join(".seat-a/settings.json"), r#"{"x":1}"#).unwrap();
    let t2 = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(!t.contains("changed") && !t2.contains("changed"), "{t}{t2}");
}

#[test]
fn default_seat_covers_unowned_folders() {
    let e = setup_with(r#"default_seat = "alpha""#);
    let t = text(&e.seat("elsewhere", &["run", "auto"]).output().unwrap());
    assert!(t.contains("SEAT=a"), "{t}");
    let t = text(&e.seat("start", &["run", "auto"]).output().unwrap());
    assert!(
        t.contains("shared start folder") && !t.contains("CFG="),
        "a shared folder still asks: {t}"
    );
}

#[test]
fn warn_lets_through_and_tells_the_user() {
    let e = setup_with(r#"enforcement = "warn""#);
    let t = text(&e.seat("proj-b", &["run", "a"]).output().unwrap());
    assert!(
        t.contains("warning") && t.contains("belongs to seat 'b'") && t.contains("SEAT=a"),
        "{t}"
    );
    let read = serde_json::json!({ "tool_name": "Read", "tool_input": { "file_path": p(&e.home.join("proj-b/x")) }, "cwd": p(&e.home.join("proj-a")) });
    let out = e.hook("tool", "a", read);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    let out = e.hook_with_dir("prompt", "a", "proj-b");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["systemMessage"]
            .as_str()
            .unwrap()
            .contains("belongs to seat 'b'"),
        "{v}"
    );
}

#[test]
fn install_hooks_removes_them_when_enforcement_is_turned_off() {
    let e = setup();
    let settings = e.home.join(".seat-a/settings.json");
    fs::write(&settings, r#"{"model":"keep-me"}"#).unwrap();
    assert!(
        e.seat("", &["install-hooks", "a"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        fs::read_to_string(&settings)
            .unwrap()
            .contains("hook prompt")
    );
    let cfg = fs::read_to_string(&e.config)
        .unwrap()
        .replace(r#"enforcement = "strict""#, "");
    fs::write(&e.config, cfg).unwrap();
    assert!(
        e.seat("", &["install-hooks", "a"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(v, serde_json::json!({ "model": "keep-me" }));
}

#[test]
fn browser_is_only_redirected_for_seats_that_want_it() {
    let e = setup_with("");
    let t = text(
        &e.seat("proj-a", &["run", "a"])
            .env_remove("BROWSER")
            .output()
            .unwrap(),
    );
    assert!(
        t.contains("BROWSER=\n"),
        "system browser: BROWSER untouched: {t}"
    );
    let t = text(&e.seat("proj-b", &["run", "b"]).output().unwrap());
    assert!(
        t.contains(&format!("BROWSER={}", env!("CARGO_BIN_EXE_seat"))),
        "copy mode: {t}"
    );
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

#[test]
fn add_pin_link_and_list() {
    let e = setup_with("");
    let ok = |args: &[&str], dir: &str| {
        let out = e.seat(dir, args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", text(&out));
        text(&out)
    };
    ok(&["add", "c", "--alias", "cc"], "");
    assert!(e.home.join(".claude-c").is_dir());
    assert!(
        !e.seat("", &["add", "cc"])
            .output()
            .unwrap()
            .status
            .success(),
        "alias taken"
    );
    fs::write(
        e.home.join(".claude-c/.claude.json"),
        r#"{"oauthAccount":{"organizationUuid":"org-c","organizationName":"Org C","emailAddress":"c@example.org"}}"#,
    )
    .unwrap();
    sign_in(&e.home.join(".claude-c"));
    assert!(ok(&["pin", "cc"], "").contains("c@example.org"));
    assert!(read(&e.config).contains(r#"email = "c@example.org""#));

    assert!(ok(&["link", "cc"], "elsewhere").ends_with(": c\n"));
    assert!(ok(&["owner"], "elsewhere").contains(": c"));
    assert!(
        read(&e.config).contains(r#"path = "~/elsewhere""#),
        "{}",
        read(&e.config)
    );
    ok(&["unlink"], "elsewhere");
    assert!(ok(&["owner"], "elsewhere").contains("unassigned"));
    assert!(
        !e.seat("elsewhere", &["link", "ghost"])
            .output()
            .unwrap()
            .status
            .success()
    );

    let list = ok(&["list"], "");
    assert!(
        list.contains("c (cc)") && list.contains("c@example.org (Org C)"),
        "{list}"
    );
    assert!(
        list.lines()
            .any(|l| l.starts_with("b ") && l.contains("WRONG")),
        "{list}"
    );
}

#[test]
fn add_from_copies_setup_but_not_identity() {
    let e = setup();
    fs::write(
        e.home.join(".seat-a/settings.json"),
        r#"{"model":"m","hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"\"/x/seat\" hook prompt a"}]}]}}"#,
    )
    .unwrap();
    fs::create_dir_all(e.home.join(".seat-a/skills/s")).unwrap();
    fs::write(e.home.join(".seat-a/skills/s/SKILL.md"), "skill").unwrap();
    let out = e.seat("", &["add", "d", "--from", "a"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let d = e.home.join(".claude-d");
    assert!(d.join("skills/s/SKILL.md").exists());
    assert!(
        !d.join(".claude.json").exists(),
        "the login stays with seat a"
    );
    let settings = read(&d.join("settings.json"));
    assert!(settings.contains(r#""model": "m""#), "{settings}");
    assert!(
        settings.contains("hook prompt d") && !settings.contains("hook prompt a"),
        "strict: seat d gets its own hooks, not a's: {settings}"
    );
}

#[test]
fn doctor_finds_missing_hooks_wrong_org_and_leaks_and_fixes_hooks() {
    let e = setup();
    fs::create_dir_all(e.home.join(".claude")).unwrap();
    fs::write(e.home.join(".claude/CLAUDE.md"), "instructions").unwrap();
    let out = e.seat("", &["doctor"]).output().unwrap();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(
        t.contains("hooks are not installed") && t.contains("[--fix]"),
        "{t}"
    );
    assert!(t.contains("another org"), "{t}");
    assert!(
        t.contains("CLAUDE.md sits above folders of seat(s) a, b"),
        "{t}"
    );

    let t = text(&e.seat("", &["doctor", "--fix"]).output().unwrap());
    assert!(t.contains("fixed: seat 'a': hooks installed"), "{t}");
    assert!(!t.contains("hooks are not installed"), "{t}");
    assert!(read(&e.home.join(".seat-b/settings.json")).contains("hook tool b"));
}

#[test]
fn init_proposes_seats_pins_and_folders_from_history() {
    let e = setup_with("");
    let session = |seat_dir: &str, project: &str, cwd: &Path| {
        let dir = e.home.join(seat_dir).join("projects").join(project);
        fs::create_dir_all(&dir).unwrap();
        let line = serde_json::json!({ "type": "user", "cwd": p(cwd) });
        fs::write(
            dir.join("s.jsonl"),
            format!("{{\"type\":\"summary\"}}\n{line}\n"),
        )
        .unwrap();
    };
    fs::create_dir_all(e.home.join(".claude-x")).unwrap();
    fs::write(
        e.home.join(".claude-x/.claude.json"),
        r#"{"oauthAccount":{"organizationUuid":"org-x","emailAddress":"x@example.org"}}"#,
    )
    .unwrap();
    session(".claude-x", "p1", &e.home.join("proj-a"));
    session(".claude-x", "p2", &e.home.join("proj-a/nested"));
    session(".claude-y", "p1", &e.home.join("proj-a"));
    session(".claude-y", "p2", &e.home.join("proj-b"));
    let target = e.home.join("fresh/config.toml");
    let out = e
        .seat("", &["init"])
        .env("SEATWISE_CONFIG", &target)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let cfg = read(&target);
    assert!(
        cfg.contains(r#"name = "x""#) && cfg.contains(r#"email = "x@example.org""#),
        "{cfg}"
    );
    assert!(
        cfg.contains(r#"name = "y""#) && cfg.contains("not signed in"),
        "{cfg}"
    );
    assert!(cfg.contains("Used by x and y"), "{cfg}");
    assert!(
        cfg.contains("version = 1") && cfg.contains("enforcement = \"warn\""),
        "a new config is versioned and isolated: {cfg}"
    );
    assert!(cfg.contains("path = \"~/proj-b\"\nowner = \"y\""), "{cfg}");
    assert!(!cfg.contains("nested"), "covered by proj-a: {cfg}");
    // Claude Code's default folder next to named ones: proposed commented out, its folders left out.
    session(".claude", "p3", &e.home.join("start"));
    let out = e
        .seat("", &["init", "--print"])
        .env("SEATWISE_CONFIG", e.home.join("none.toml"))
        .output()
        .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("~/.claude is the agent's default folder")
            && t.contains("# [[seat]]\n# name = \"default\""),
        "{t}"
    );
    assert!(!t.contains("owner = \"default\""), "{t}");
    let proposal: seatwise_config_probe::Probe = toml::from_str(&t).unwrap();
    assert!(proposal.seat.iter().all(|s| s.name != "default"), "{t}");
    // An existing config is never overwritten without --force.
    let out = e
        .seat("", &["init"])
        .env("SEATWISE_CONFIG", &target)
        .output()
        .unwrap();
    assert!(text(&out).contains("already exists"), "{}", text(&out));
}

/// Just enough of a config to read seat names from a proposal.
mod seatwise_config_probe {
    #[derive(serde::Deserialize)]
    pub struct Probe {
        #[serde(default)]
        pub seat: Vec<Named>,
    }
    #[derive(serde::Deserialize)]
    pub struct Named {
        pub name: String,
    }
}

#[test]
fn folder_lists_and_missing_folders_are_handled_without_naming_paths() {
    let e = setup_with("");
    let list = e.home.join("projects.txt");
    fs::write(
        &list,
        "# owner path\nalpha ~/elsewhere\n\nnone ~/closed/deep\nany ~/start\n",
    )
    .unwrap();
    let t = ok(&e
        .seat("", &["link", "--from", list.to_str().unwrap()])
        .output()
        .unwrap());
    assert!(
        t.contains("elsewhere: a") && t.contains("deep: none"),
        "{t}"
    );
    assert!(read(&e.config).contains("path = \"~/elsewhere\"\nowner = \"a\""));
    // One bad line: nothing is written.
    fs::write(&list, "a ~/proj-b\nnobody ~/x\nb relative/path\n").unwrap();
    let out = e
        .seat("", &["link", "--from", list.to_str().unwrap()])
        .output()
        .unwrap();
    let t = text(&out);
    assert!(!out.status.success(), "{t}");
    assert!(
        t.contains("line 2: unknown seat 'nobody'") && t.contains("line 3:"),
        "{t}"
    );
    assert!(!read(&e.config).contains("path = \"~/proj-b\"\nowner = \"a\""));

    // A folder that is gone: doctor points at --missing, and --missing removes it.
    fs::remove_dir_all(e.home.join("elsewhere")).unwrap();
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(t.contains("unlink --missing"), "{t}");
    let t = ok(&e.seat("", &["unlink", "--missing"]).output().unwrap());
    assert!(t.contains("~/elsewhere: entry removed"), "{t}");
    assert!(!read(&e.config).contains("~/elsewhere"));
    assert!(
        ok(&e.seat("", &["unlink", "--missing"]).output().unwrap())
            .contains("every folder entry exists")
    );
}

#[test]
fn a_session_started_from_inside_another_drops_its_wiring() {
    let e = setup();
    let t = ok(&e
        .seat("proj-a", &["run", "a"])
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:9/desktop")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "x")
        .output()
        .unwrap());
    assert!(
        t.contains("BASE=http://127.0.0.1:9/desktop"),
        "your own base URL stays: {t}"
    );
    let t = ok(&e
        .seat("proj-a", &["run", "a"])
        .env("CLAUDE_CODE_ENTRYPOINT", "claude-desktop")
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:9/desktop")
        .output()
        .unwrap());
    assert!(
        t.contains("BASE=\n"),
        "the enclosing session's base URL is dropped: {t}"
    );
}

#[test]
fn login_runs_claude_auth_login_in_the_seat() {
    let e = setup();
    let t = text(&e.seat("elsewhere", &["login", "alpha"]).output().unwrap());
    assert!(t.contains("ARGS=auth login"), "{t}");
    assert!(
        t.contains(&format!("CFG={}", p(&e.home.join(".seat-a")))),
        "{t}"
    );
}

#[test]
fn shared_items_are_copied_on_start() {
    let e = setup();
    let cfg = read(&e.config).replace(
        "org_uuid = \"org-a\"",
        "org_uuid = \"org-a\"\n        share = [\"skills/s1\"]",
    );
    fs::write(&e.config, cfg).unwrap();
    fs::create_dir_all(e.home.join("shared/skills/s1")).unwrap();
    fs::write(e.home.join("shared/skills/s1/SKILL.md"), "v1").unwrap();
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("updated 1 file(s) of shared 'skills/s1'"), "{t}");
    assert_eq!(read(&e.home.join(".seat-a/skills/s1/SKILL.md")), "v1");
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(!t.contains("updated"), "unchanged: nothing to copy: {t}");
}

#[test]
fn shim_completions_and_ps() {
    let e = setup_with("");
    let out = e.seat("", &["shim", "install"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let shim = e.home.join("data/bin").join(if cfg!(windows) {
        "claude.cmd"
    } else {
        "claude"
    });
    assert!(read(&shim).contains("run auto"), "{}", read(&shim));
    assert!(
        text(&e.seat("", &["doctor"]).output().unwrap()).contains("not ahead of the real claude")
    );
    assert!(text(&e.seat("", &["shim", "uninstall"]).output().unwrap()).contains("removed"));
    assert!(!shim.exists());

    let out = e.seat("", &["completions", "zsh"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("#compdef seat"));
    let out = e.seat("", &["ps"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
}

/// One-shot HTTP server answering `body`; returns its URL and the request it saw.
fn serve_once(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/oauth/usage", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
            request += &line;
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        request
    });
    (url, handle)
}

fn team_account(seat_tier: &str) -> String {
    format!(
        r#"{{"oauthAccount":{{"organizationUuid":"org-a","emailAddress":"a@example.org","organizationType":"claude_team","organizationRateLimitTier":"default_x","seatTier":"{seat_tier}","hasExtraUsageEnabled":true}}}}"#
    )
}

#[test]
fn plan_shows_in_list_and_profiles_apply_without_touching_hooks() {
    let e = setup();
    fs::write(
        e.home.join(".seat-a/.claude.json"),
        team_account("team_premium"),
    )
    .unwrap();
    assert!(
        e.seat("", &["install-hooks", "a"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(e.seat("", &["pin", "a"]).output().unwrap().status.success());
    assert!(
        read(&e.config).contains(r#"plan = "team_premium""#),
        "{}",
        read(&e.config)
    );

    let list = text(&e.seat("", &["list"]).output().unwrap());
    assert!(list.contains("Team premium +extra"), "{list}");
    let t = text(&e.seat("", &["profile", "a"]).output().unwrap());
    assert!(t.contains("suggested: balanced (Team premium plan)"), "{t}");

    let out = e.seat("", &["profile", "a", "light"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let settings = read(&e.home.join(".seat-a/settings.json"));
    assert!(settings.contains(r#""model": "sonnet""#), "{settings}");
    assert!(
        settings.contains("hook tool a"),
        "guard hooks stay: {settings}"
    );
    assert!(read(&e.config).contains(r#"profile = "light""#));
    assert!(
        !e.seat("", &["profile", "a", "nope"])
            .output()
            .unwrap()
            .status
            .success()
    );

    // A plan change since the pin is reported.
    fs::write(
        e.home.join(".seat-a/.claude.json"),
        team_account("team_standard"),
    )
    .unwrap();
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(
        t.contains("plan changed from team_premium to team_standard"),
        "{t}"
    );
    assert!(t.contains("extra usage is on"), "{t}");
}

#[test]
fn usage_reads_the_seats_own_login_and_never_prints_it() {
    let e = setup();
    fs::write(
        e.home.join(".seat-a/.credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"tok-secret-a","expiresAt":99999999999999}}"#,
    )
    .unwrap();
    let (url, server) = serve_once(
        r#"{"five_hour":{"utilization":20.0,"resets_at":"2026-11-22T14:05:00Z"},"seven_day":{"utilization":81.0},"extra_usage":{"is_enabled":true}}"#,
    );
    let out = e
        .seat("", &["usage", "a"])
        .env("SEATWISE_USAGE_URL", &url)
        .output()
        .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("5h") && t.contains("20%") && t.contains("resets 2026-11-22 14:05 UTC"),
        "{t}"
    );
    assert!(t.contains("7d") && t.contains("81%"), "{t}");
    assert!(!t.contains("tok-secret-a"), "the token is never shown: {t}");
    let request = server.join().unwrap().to_ascii_lowercase();
    assert!(
        request.contains("authorization: bearer tok-secret-a"),
        "{request}"
    );
    assert!(
        request.contains("anthropic-beta: oauth-2025-04-20"),
        "{request}"
    );

    // An expired login is not refreshed (that would rotate Claude Code's token).
    fs::write(
        e.home.join(".seat-a/.credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"t","expiresAt":1}}"#,
    )
    .unwrap();
    let t = text(&e.seat("", &["usage", "a"]).output().unwrap());
    assert!(t.contains("expired"), "{t}");
}

#[test]
fn term_prints_the_command_for_the_desktop_project_and_seat() {
    let e = setup_with(r#"terminal.profile = "light""#);
    // As a desktop or CLI session of seat a would call it from its skill.
    let out = e
        .seat("", &["term", "--print"])
        .env("CLAUDE_CONFIG_DIR", e.home.join(".seat-a"))
        .env("CLAUDE_PROJECT_DIR", e.home.join("proj-a"))
        .output()
        .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains(&p(&e.home.join("proj-a"))), "{t}");
    assert!(
        t.contains("'--with' 'light' 'a'") || t.contains(r#""--with" "light" "a""#),
        "{t}"
    );

    // The desktop app only exports its org: the seat pinned to it.
    let t = text(
        &e.seat(
            "proj-a",
            &["term", "--print", "--with", "heavy", "--", "--continue"],
        )
        .env("CLAUDE_CODE_ORGANIZATION_UUID", "org-a")
        .output()
        .unwrap(),
    );
    assert!(
        t.contains("heavy") && (t.contains("'a'") || t.contains("\"a\"")),
        "{t}"
    );
    assert!(t.contains("--continue") && !t.contains("light"), "{t}");
}

#[test]
fn run_with_a_profile_passes_session_settings_only() {
    let e = setup();
    let out = e
        .seat("proj-a", &["run", "--with", "light", "a"])
        .output()
        .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("--settings") && t.contains(r#""model":"sonnet""#),
        "{t}"
    );
    assert!(
        !fs::read_to_string(e.home.join(".seat-a/settings.json"))
            .unwrap_or_default()
            .contains("sonnet"),
        "session only"
    );
    assert!(
        !e.seat("proj-a", &["run", "--with", "nope", "a"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn home_repo_points_only_to_the_seats_own_repos() {
    let e = setup();
    for repo in ["proj-a", "proj-a/nested", "proj-b"] {
        fs::create_dir_all(e.home.join(repo).join(".git")).unwrap();
    }
    fs::write(e.home.join("proj-a/README.md"), "# Alpha service\n").unwrap();
    let out = e.seat("", &["home", "init", "a"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let home = e.home.join("a-home");
    let agents = read(&home.join("AGENTS.md"));
    assert!(
        agents.contains("- proj-a (Alpha service): ~/proj-a."),
        "{agents}"
    );
    assert!(
        !agents.contains("nested") && !agents.contains("proj-b"),
        "{agents}"
    );
    assert_eq!(read(&home.join("CLAUDE.md")), "@AGENTS.md\n");
    assert!(read(&e.config).contains(r#"home = "~/a-home""#));

    // --home starts in it, and its skills reach the seat.
    fs::create_dir_all(home.join("skills/mine")).unwrap();
    fs::write(home.join("skills/mine/SKILL.md"), "---\nname: mine\n---\n").unwrap();
    let out = e.seat("proj-a", &["run", "--home", "a"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert!(e.home.join(".seat-a/skills/mine/SKILL.md").is_file());

    // Mentioning another seat's folder is flagged.
    fs::write(home.join("AGENTS.md"), agents + "\nAlso see ~/proj-b\n").unwrap();
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(t.contains("home repo mentions ~/proj-b"), "{t}");
}

#[test]
fn skills_install_into_the_seat_with_the_binary_path() {
    let e = setup();
    let out = e.seat("", &["skills", "install", "a"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let out = text(&out);
    assert!(out.contains("/seatwise:hybrid"), "{out}");
    let plugin = e.home.join(".seat-a/seatwise-plugins");
    let skill = read(&plugin.join("plugins/seatwise/skills/terminal/SKILL.md"));
    assert!(skill.starts_with("---\nname: terminal"), "{skill}");
    assert!(plugin.join(".claude-plugin/marketplace.json").is_file());
    let settings = read(&e.home.join(".seat-a/settings.json"));
    assert!(
        settings.contains("\"seatwise@seatwise\": true"),
        "{settings}"
    );
    // Registered through `claude plugin`, in the seat's own folder.
    assert!(
        e.home
            .join(".seat-a/plugins/installed_plugins.json")
            .is_file()
    );
    assert!(
        !skill.contains("{{seat}}") && skill.contains(env!("CARGO_BIN_EXE_seat")),
        "{skill}"
    );
    assert!(
        !e.home.join(".seat-b/seatwise-plugins").exists(),
        "only the seat asked for"
    );
    let t = text(
        &e.seat("", &["skills", "show", "/seatwise:home"])
            .output()
            .unwrap(),
    );
    assert!(t.contains("name: home"), "{t}");
}

#[test]
fn codex_seats_get_their_own_home_hooks_and_guard() {
    let e = setup();
    fs::create_dir_all(e.home.join(".seat-c")).unwrap();
    fs::create_dir_all(e.home.join("proj-c")).unwrap();
    let mut cfg = read(&e.config);
    cfg += "\n[[seat]]\nname = \"c\"\nagent = \"codex\"\nconfig_dir = \"~/.seat-c\"\n[[folder]]\npath = \"~/proj-c\"\nowner = \"c\"\n";
    fs::write(&e.config, cfg).unwrap();
    let fake_codex = e
        .home
        .join(format!("fake-codex{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_seat"), &fake_codex).unwrap();

    let out = e
        .seat("proj-c", &["run", "auto"])
        .env("SEATWISE_CODEX", &fake_codex)
        .env("OPENAI_API_KEY", "leaked")
        .output()
        .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains(&format!("CODEX={}", p(&e.home.join(".seat-c")))),
        "{t}"
    );
    assert!(t.contains("CFG=\n") || t.contains("CFG=\r\n"), "{t}");
    assert!(t.contains("SEAT=c"), "{t}");

    let out = e.seat("", &["install-hooks", "c"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let hooks = read(&e.home.join(".seat-c/hooks.json"));
    assert!(
        hooks.contains("hook tool c") && hooks.contains("UserPromptSubmit"),
        "{hooks}"
    );
    assert!(!e.home.join(".seat-c/settings.json").exists());

    // An apply_patch into another seat's folder is blocked; its own folder is fine.
    let patch = |file: &str| {
        serde_json::json!({
            "tool_name": "apply_patch",
            "cwd": p(&e.home.join("proj-c")),
            "tool_input": { "command": format!("*** Begin Patch\n*** Update File: {file}\n@@\n-a\n+b\n*** End Patch\n") }
        })
    };
    let out = e.hook("tool", "c", patch(&p(&e.home.join("proj-b/x.rs"))));
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    let out = e.hook("tool", "c", patch("src/main.rs"));
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));

    let list = text(&e.seat("", &["list"]).output().unwrap());
    assert!(list.contains("c [codex]"), "{list}");
    let t = text(&e.seat("", &["usage", "c"]).output().unwrap());
    assert!(t.contains("runs Codex"), "{t}");
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(t.contains("/hooks"), "{t}");
}

fn ok(o: &Output) -> String {
    let t = text(o);
    assert!(o.status.success(), "{t}");
    t
}

#[test]
fn a_recorded_account_without_a_login_is_signed_out() {
    let e = setup_with("");
    fs::remove_file(e.home.join(".seat-a/.credentials.json")).unwrap();
    let list = ok(&e.seat("", &["list"]).output().unwrap());
    let row = list.lines().find(|l| l.starts_with("a ")).unwrap();
    assert!(
        row.contains("signed out (login gone)") && row.contains("SIGNED OUT"),
        "{list}"
    );
    assert!(
        !row.contains(" ok "),
        "not 'identity ok' without a login: {list}"
    );
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(
        t.contains("seat 'a' is signed out") && t.contains("`seat login a`"),
        "{t}"
    );
    // Claude Code's default folder coming back while no seat uses it is reported.
    assert!(!t.contains("no seat uses it"), "{t}");
    // Only folders the desktop app or a tool left behind (empty ones, a skill) are an info line, not a warning.
    fs::create_dir_all(e.home.join(".claude/bridge-spawn")).unwrap();
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(
        t.contains("info   ~/.claude holds no settings or sessions, only: bridge-spawn")
            && !t.contains("no seat uses it"),
        "{t}"
    );
    fs::write(e.home.join(".claude/CLAUDE.md"), "x").unwrap();
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(
        t.contains("warn   ~/.claude holds settings or sessions but no seat uses it"),
        "{t}"
    );
}

#[test]
fn desktop_starts_each_seat_once_with_its_own_data_folder_and_environment() {
    let e = setup_with("");
    let ps = e.home.join("ps.txt");
    let support = e.home.join("Library/Application Support");
    let desktop = |args: &[&str], processes: &str| {
        fs::write(&ps, processes).unwrap();
        e.seat("", args)
            .env("SEATWISE_OPEN", &e.fake_claude)
            .env("SEATWISE_DESKTOP_PS", &ps)
            .output()
            .unwrap()
    };
    let app = "/Applications/Claude.app/Contents/MacOS/Claude";

    // Nothing runs: start it, with the seat's own data folder and environment.
    let t = ok(&desktop(&["desktop", "b"], ""));
    assert!(
        t.contains(&format!(
            "ARGS=-n -a /Applications/Claude.app --env CLAUDE_CONFIG_DIR={} --env SEATWISE_SEAT=b --env BROWSER=",
            e.home.join(".seat-b").display()
        )),
        "{t}"
    );
    assert!(
        t.contains(&format!(
            "--args --user-data-dir={}",
            support.join("Claude-b").display()
        )),
        "{t}"
    );
    assert!(t.contains("started the desktop app on seat 'b'"), "{t}");
    assert!(!t.contains("note:"), "nothing else runs: {t}");

    // Already running on that data folder: never a second instance.
    let running_b = format!(
        "100\tSun Nov 22 22:43:01 2026\t{app} --user-data-dir={}\tSEATWISE_SEAT=b CLAUDE_CONFIG_DIR={} BROWSER=/old/copy-link\n",
        support.join("Claude-b").display(),
        e.home.join(".seat-b").display()
    );
    let t = ok(&desktop(&["desktop", "b"], &running_b));
    assert!(t.contains("already running (pid 100"), "{t}");
    assert!(!t.contains("ARGS="), "{t}");
    assert!(t.contains("older setup (BROWSER differ)"), "{t}");

    // A first sign-in while another instance runs: say how to avoid the stuck Google sign-in.
    let t = ok(&desktop(&["desktop", "a"], &running_b));
    assert!(
        t.contains("note: seat 'a' has no desktop data folder yet") && t.contains("email"),
        "{t}"
    );
    assert!(t.contains("ARGS=-n -a"), "{t}");

    // Doctor: an instance without a seat, a stale one, and two on one data folder.
    let dock = format!("200\tSun Nov 22 23:00:00 2026\t{app}\tPATH=/usr/bin\n");
    let t = text(&desktop(
        &["doctor"],
        &format!("{running_b}{dock}{running_b}"),
    ));
    assert!(t.contains("runs without a seat (pid 200"), "{t}");
    assert!(t.contains("restart the 'b' desktop"), "{t}");
    assert!(
        t.contains("two Claude desktop instances share one data folder"),
        "{t}"
    );

    // The list of running instances.
    let t = ok(&desktop(&["desktop"], &format!("{running_b}{dock}")));
    assert!(
        t.contains("seat b") && t.contains("no seat") && t.contains("main.log"),
        "{t}"
    );

    // The app's default data folder: no --user-data-dir.
    let mut cfg = read(&e.config);
    cfg = cfg.replacen(
        "name = \"a\"",
        "name = \"a\"\ndesktop = { user_data_dir = \"default\" }",
        1,
    );
    fs::write(&e.config, cfg).unwrap();
    let t = ok(&desktop(&["desktop", "a"], ""));
    assert!(
        t.contains("ARGS=-n -a") && !t.contains("--user-data-dir"),
        "{t}"
    );

    // Opened from the Dock on that folder: the seat's desktop login, but Code sessions from ~/.claude, so the
    // seat's sessions look gone. Say so and how to get them back, instead of "already running".
    let out = desktop(&["desktop", "a"], &dock);
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(
        t.contains("seat 'a''s data folder runs without the seat (pid 200")
            && t.contains("Nothing is lost")
            && t.contains("resolution: Quit that window (Cmd-Q), then: seat desktop a"),
        "{t}"
    );
    assert!(!t.contains("ARGS="), "{t}");
    let t = text(&desktop(&["doctor"], &dock));
    assert!(
        t.contains("seat 'a''s data folder runs without the seat")
            && t.contains("run `seat desktop a`"),
        "{t}"
    );
    let t = ok(&desktop(&["desktop"], &dock));
    assert!(t.contains("no seat (on a's data)"), "{t}");

    // A shell command per seat.
    let sh = ok(&e
        .seat("", &["shell-init", "zsh"])
        .env("SEATWISE_DESKTOP_SHELL", "1")
        .output()
        .unwrap());
    assert!(
        sh.contains("function claude-desktop-a {") && sh.contains("desktop a; }"),
        "{sh}"
    );
}

#[test]
fn a_seat_whose_desktop_runs_in_the_default_folder_is_told_to_keep_it() {
    // Set up before seatwise (or by hand): the app runs in its default data folder, with the seat's config folder.
    let e = setup_with("");
    let ps = e.home.join("ps.txt");
    fs::write(
        &ps,
        format!(
            "300\tSun Nov 22 23:02:10 2026\t/Applications/Claude.app/Contents/MacOS/Claude\tPATH=/usr/bin CLAUDE_CONFIG_DIR={}\n",
            e.home.join(".seat-a").display()
        ),
    )
    .unwrap();
    let run = |args: &[&str]| {
        e.seat("", args)
            .env("SEATWISE_OPEN", &e.fake_claude)
            .env("SEATWISE_DESKTOP_PS", &ps)
            .output()
            .unwrap()
    };

    let t = ok(&run(&["desktop"]));
    assert!(
        t.contains("seat a ") && t.contains("user_data_dir = \"default\""),
        "{t}"
    );

    // Never a second, signed-out instance next to it.
    let t = ok(&run(&["desktop", "a"]));
    assert!(
        t.contains("already runs in the app's default data folder"),
        "{t}"
    );
    assert!(!t.contains("ARGS="), "{t}");

    let t = text(&run(&["doctor"]));
    assert!(
        t.contains("seat 'a' desktop runs in the app's default data folder")
            && t.contains("[--fix]"),
        "{t}"
    );
    assert!(
        !t.contains("runs without a seat") && !t.contains("restart the"),
        "{t}"
    );

    text(&run(&["doctor", "--fix"]));
    let cfg = read(&e.config);
    assert!(
        cfg.contains("[seat.desktop]\nuser_data_dir = \"default\""),
        "{cfg}"
    );
    assert!(
        cfg.find("[seat.desktop]") < cfg.find("name = \"b\""),
        "the table sits under seat a: {cfg}"
    );
    let t = text(&run(&["doctor"]));
    assert!(!t.contains("default data folder"), "fixed: {t}");
    let t = ok(&run(&["desktop", "a"]));
    assert!(t.contains("already running (pid 300"), "{t}");
}

#[test]
fn compaction_tiers_are_enforced_per_seat() {
    let e = setup_with("");
    let t = ok(&e.seat("", &["compact", "a", "sharp"]).output().unwrap());
    assert!(
        t.contains("sharp (150K) (seat)") && t.contains("settings.json"),
        "{t}"
    );
    assert!(read(&e.config).contains("compact = \"sharp\""));
    let settings = read(&e.home.join(".seat-a/settings.json"));
    assert!(
        settings.contains("\"autoCompactWindow\": 150000"),
        "{settings}"
    );

    // Every session gets it, over project settings; `--with` decides for one session.
    let t = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("\"autoCompactWindow\":150000"), "{t}");
    let t = ok(&e
        .seat("proj-a", &["run", "--with", "lazy", "a"])
        .output()
        .unwrap());
    assert!(t.contains("\"autoCompactWindow\":1000000"), "{t}");

    // Changed by hand (or by the desktop app): put back on the next start.
    fs::write(
        e.home.join(".seat-a/settings.json"),
        "{\"autoCompactWindow\": 900000}",
    )
    .unwrap();
    let t = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("compaction at sharp (150K) set"), "{t}");

    // A tier given to `seat profile` is the seat's compaction, not its usage profile.
    let t = ok(&e.seat("", &["profile", "a", "standard"]).output().unwrap());
    assert!(t.contains("standard (400K)"), "{t}");
    assert!(!read(&e.config).contains("profile = \"standard\""));

    // The global default, a seat that opts out, bad values.
    ok(&e.seat("", &["compact", "all", "250k"]).output().unwrap());
    ok(&e.seat("", &["compact", "b", "off"]).output().unwrap());
    let t = ok(&e.seat("", &["compact"]).output().unwrap());
    assert!(
        t.contains("global   250K") && t.contains("standard (400K) (seat)"),
        "{t}"
    );
    let out = e.seat("", &["compact", "a", "50k"]).output().unwrap();
    assert!(
        !out.status.success() && text(&out).contains("100000"),
        "{}",
        text(&out)
    );
}

#[test]
fn the_settings_catalog_checks_presets_where_they_are_applied() {
    let e = setup_with(
        r#"[preset.fast]
        description = "code mode"
        [preset.fast.settings]
        effortLevel = "hihg"
        [preset.fast.settings.env]
        CLAUDE_CODE_EXPERIMENTAL_CODE_MODE = "1"
        [profile.disk]
        env = { CLAUDE_CODE_WARN_DISK_FREE_GIB = "5", CLAUDE_CODE_EXPERIMENTAL_CODE_MODE = "1" }
        [[group]]
        name = "g"
        paths = ["~/proj-a"]
        preset = "fast"
        apply = "project""#,
    );

    // Discovery: experimental settings, details, and typos.
    let t = ok(&e
        .seat("", &["settings", "search", "--experimental"])
        .output()
        .unwrap());
    assert!(t.contains("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE"), "{t}");
    let t = ok(&e
        .seat("", &["settings", "show", "permissions.defaultMode"])
        .output()
        .unwrap());
    assert!(t.contains("auto: only in user, managed, session"), "{t}");
    let t = text(
        &e.seat("", &["settings", "show", "efortLevel"])
            .output()
            .unwrap(),
    );
    assert!(t.contains("did you mean effortLevel?"), "{t}");

    // A bad value, and code mode in a project file, where Claude Code ignores it.
    let out = e
        .seat("", &["settings", "check", "fast", "--target", "local"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let t = text(&out);
    assert!(t.contains("effortLevel = \"hihg\" is not valid"), "{t}");
    assert!(
        t.contains("ignores CLAUDE_CODE_EXPERIMENTAL_CODE_MODE"),
        "{t}"
    );
    let t = text(&e.seat("", &["doctor"]).output().unwrap());
    assert!(
        t.contains("preset 'fast'") && t.contains("ignores CLAUDE_CODE_EXPERIMENTAL_CODE_MODE"),
        "{t}"
    );
    let t = text(&e.seat("proj-a", &["explain"]).output().unwrap());
    assert!(t.contains("from the settings catalog"), "{t}");

    // As a session profile, code mode works; a launch-only variable reaches the process environment.
    let out = e
        .seat("proj-b", &["settings", "check", "disk"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let t = text(
        &e.seat("proj-b", &["run", "b", "--with", "disk"])
            .output()
            .unwrap(),
    );
    assert!(t.contains("DISKWARN=5"), "{t}");
    assert!(
        t.contains("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE"),
        "passed in --settings: {t}"
    );
}

#[test]
fn presets_are_built_one_valid_choice_at_a_time() {
    let e = setup_with("");
    let run = |args: &[&str]| e.seat("", args).output().unwrap();

    ok(&run(&[
        "preset",
        "new",
        "fast",
        "--from",
        "build",
        "--description",
        "build, faster",
    ]));
    let t = ok(&run(&[
        "preset",
        "set",
        "fast",
        "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE",
        "1",
    ]));
    assert!(t.contains("note:") && t.contains("experimental"), "{t}");
    ok(&run(&["preset", "set", "fast", "effortLevel", "high"]));

    // Refused: a bad value, a typo, and a documented clash. Nothing is written.
    let before = read(&e.config);
    for (args, why) in [
        (
            vec!["preset", "set", "fast", "effortLevel", "hihg"],
            "not valid",
        ),
        (
            vec!["preset", "set", "fast", "efortLevel", "high"],
            "did you mean effortLevel",
        ),
        (
            vec!["preset", "set", "fast", "CLAUDE_CODE_EFFORT_LEVEL", "low"],
            "overrides effortLevel",
        ),
    ] {
        let out = run(&args);
        assert!(!out.status.success(), "{args:?}");
        assert!(text(&out).contains(why), "{args:?}: {}", text(&out));
    }
    assert_eq!(read(&e.config), before);

    let cfg = read(&e.config);
    assert!(cfg.contains("[preset.fast.settings]"), "{cfg}");
    assert!(
        cfg.contains("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE = \"1\""),
        "{cfg}"
    );
    let t = ok(&run(&["preset", "options", "fast", "effort"]));
    assert!(
        t.contains("effortLevel") && t.contains("[now \"high\"]"),
        "{t}"
    );

    // The wizard, driven through stdin: add a description and remove a setting, then write.
    let mut child = e
        .seat("", &["preset", "edit", "fast"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"r\neffortLevel\nd\nbuild with code mode\nw\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("Only settings Claude Code honors"), "{t}");
    let cfg = read(&e.config);
    assert!(
        cfg.contains("build with code mode") && !cfg.contains("effortLevel"),
        "{cfg}"
    );

    // A project preset can't take what project files ignore.
    ok(&run(&["preset", "new", "plain"]));
    let mut cfg = read(&e.config);
    cfg += "\n[[group]]\nname = \"g\"\npaths = [\"~/proj-a\"]\npreset = \"plain\"\napply = \"project\"\n";
    fs::write(&e.config, cfg).unwrap();
    let out = run(&[
        "preset",
        "set",
        "plain",
        "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE",
        "1",
    ]);
    assert!(text(&out).contains("ignores"), "{}", text(&out));
    let t = text(&run(&["doctor"]));
    assert!(
        !t.contains("preset 'fast'") && !t.contains("preset 'plain'"),
        "{t}"
    );
}

#[test]
fn a_seat_and_its_projects_in_one_view() {
    let e = setup();
    let t = ok(&e.seat("", &["show", "a"]).output().unwrap());
    assert!(
        t.contains("a · claude (alpha)") && t.contains("identity"),
        "{t}"
    );
    assert!(t.contains("projects (") && t.contains("~/proj-a"), "{t}");
    let t = ok(&e.seat("", &["show", "b"]).output().unwrap());
    assert!(t.contains("[wrong account]"), "{t}");

    let t = ok(&e.seat("", &["projects"]).output().unwrap());
    assert!(
        t.contains("PATH") && t.contains("SEAT") && t.contains("~/proj-b"),
        "{t}"
    );
    let json: serde_json::Value =
        serde_json::from_str(&ok(&e.seat("", &["projects", "--json"]).output().unwrap())).unwrap();
    assert!(json.as_array().is_some_and(|a| !a.is_empty()), "{json}");

    // An agent reading the JSON gets no identifiers.
    let t = ok(&e
        .seat("", &["show", "a", "--json"])
        .env("CLAUDECODE", "1")
        .output()
        .unwrap());
    assert!(!t.contains("org-a"), "{t}");
}

#[test]
fn shell_init_replaces_aliases_of_the_same_name() {
    // Old setups often have `alias claude-work=...`: re-sourcing shell-init over them must still define functions.
    // Unix only: on Windows `bash` may be the WSL launcher, which can't read this test's Windows paths.
    if !cfg!(unix) {
        return;
    }
    let e = setup();
    let file = e.home.join("init.sh");
    fs::write(
        &file,
        ok(&e.seat("", &["shell-init", "zsh"]).output().unwrap()),
    )
    .unwrap();
    let script = |prelude: &str, query: &str| {
        format!(
            "{prelude}alias claude-a='echo old'; alias claude-alpha='echo old'\neval \"$(cat '{}')\"\n{query} claude-a; {query} claude-alpha",
            file.display()
        )
    };
    let shells = [
        ("zsh", vec!["-f", "-c"], "", "whence -w"),
        ("bash", vec!["-c"], "shopt -s expand_aliases\n", "type -t"),
    ];
    let mut ran = 0;
    for (shell, args, prelude, query) in shells {
        let Ok(out) = Command::new(shell)
            .args(&args)
            .arg(script(prelude, query))
            .output()
        else {
            continue; // shell not installed
        };
        ran += 1;
        let t = text(&out);
        assert!(out.status.success(), "{shell}: {t}");
        assert!(!t.contains("alias"), "{shell} kept the alias: {t}");
        assert_eq!(t.matches("function").count(), 2, "{shell}: {t}");
    }
    assert!(ran > 0, "no shell to test with");
}

#[test]
fn disabled_seats_and_agents_cannot_start() {
    let e = setup();
    ok(&e.seat("", &["disable", "a"]).output().unwrap());
    assert!(read(&e.config).contains("enabled = false"));
    let t = text(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("seat 'a' can't start: it is disabled"), "{t}");
    let t = text(&e.seat("proj-a", &["run", "auto"]).output().unwrap());
    assert!(t.contains("which is disabled"), "{t}");
    let sh = ok(&e.seat("", &["shell-init", "zsh"]).output().unwrap());
    assert!(
        !sh.contains("function claude-a ") && sh.contains("function claude-b "),
        "{sh}"
    );
    assert!(ok(&e.seat("", &["list"]).output().unwrap()).contains("a (alpha) (off)"));
    ok(&e.seat("", &["enable", "seat:a"]).output().unwrap());
    ok(&e.seat("", &["disable", "agent:claude"]).output().unwrap());
    let t = text(&e.seat("proj-b", &["run", "b"]).output().unwrap());
    assert!(t.contains("the claude agent is turned off"), "{t}");
    ok(&e.seat("", &["enable", "agent:claude"]).output().unwrap());
    ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(
        !e.seat("", &["disable", "nothing"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn folder_groups_apply_presets_per_session_or_into_the_project() {
    let e = setup_with(""); // seat b's wrong org only warns here
    let add = |dir: &str, extra: &[&str]| {
        let mut args = vec!["group", "add"];
        args.extend_from_slice(extra);
        ok(&e.seat(dir, &args).output().unwrap());
    };
    add("proj-a", &["--preset", "review", "--sources", "user,local"]);
    add("proj-b", &["--preset", "build", "--apply", "project"]);
    add("proj-a/nested", &["--name", "nested", "--preset", "none"]);

    let t = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(
        t.contains(r#""defaultMode":"plan""#) && t.contains(r#""model":"opus""#),
        "{t}"
    );
    assert!(t.contains("--setting-sources user,local"), "{t}");
    let print = t
        .lines()
        .find(|l| l.starts_with("PRESET=review@"))
        .unwrap()
        .to_string();
    let again = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(
        again.contains(&print),
        "same folder, same fingerprint: {again}"
    );

    // Project mode writes once, then leaves the file alone.
    let t = ok(&e.seat("proj-b", &["run", "b"]).output().unwrap());
    assert!(!t.contains("--settings"), "{t}");
    assert!(t.contains("preset 'build' written"), "{t}");
    let local = read(&e.home.join("proj-b/.claude/settings.local.json"));
    assert!(
        local.contains("acceptEdits") && local.contains("opusplan"),
        "{local}"
    );
    let t = ok(&e.seat("proj-b", &["run", "b"]).output().unwrap());
    assert!(!t.contains("written"), "{t}");

    // A nested opt-out, and explain.
    let t = ok(&e.seat("proj-a/nested", &["run", "b"]).output().unwrap());
    assert!(t.contains("PRESET=-@") && !t.contains("--settings"), "{t}");
    let t = ok(&e.seat("proj-a", &["explain"]).output().unwrap());
    assert!(t.contains("preset   review (apply: session)"), "{t}");
    assert!(
        t.contains("permissions.defaultMode") && t.contains("← preset review"),
        "{t}"
    );
    assert!(t.contains("sources  user, local"), "{t}");

    // Turning the preset off falls back to nothing; the list shows it.
    ok(&e.seat("", &["disable", "preset:review"]).output().unwrap());
    let t = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(!t.contains("--settings"), "{t}");
    let t = ok(&e.seat("", &["preset", "list"]).output().unwrap());
    assert!(
        t.lines()
            .any(|l| l.contains("review") && l.contains(" no ")),
        "{t}"
    );
    assert!(
        !e.seat("", &["group", "add", "--preset", "nope"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn sentinel_reports_once_per_interval_and_can_be_silenced() {
    let e = setup_with(
        r#"[sentinel]
        enabled = true
        interval = "1d"
        notify = false"#,
    );
    fs::create_dir_all(e.home.join("proj-a/.git")).unwrap();
    fs::write(e.home.join("proj-a/CLAUDE.md"), "x".repeat(45_000)).unwrap();

    let t = ok(&e.seat("", &["sentinel"]).output().unwrap());
    assert!(t.contains("[instructions-size] ~/proj-a"), "{t}");
    let t = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(t.contains("sentinel: 1 suggestion"), "{t}");
    let t = ok(&e.seat("proj-a", &["run", "a"]).output().unwrap());
    assert!(!t.contains("sentinel"), "once per interval: {t}");

    let cfg = read(&e.config).replace(
        "notify = false",
        "notify = false\nignore = [\"instructions-size@~/proj-a\"]",
    );
    fs::write(&e.config, cfg).unwrap();
    let t = ok(&e.seat("", &["sentinel"]).output().unwrap());
    assert!(t.contains("no suggestions"), "{t}");

    let t = ok(&e
        .seat("", &["sentinel", "install", "--dry-run"])
        .output()
        .unwrap());
    assert!(t.contains("sentinel") && t.contains("--notify"), "{t}");
    assert!(t.contains("86400") || t.contains("DAILY"), "{t}");
    let t = ok(&e.seat("", &["sentinel", "rules"]).output().unwrap());
    assert!(t.contains("mcp-unused"), "{t}");
}

#[test]
fn identifiers_are_hidden_from_agents_and_pipes() {
    let e = setup();
    ok(&e.seat("", &["pin", "a"]).output().unwrap());
    let shown = ok(&e.seat("", &["list"]).output().unwrap());
    assert!(
        shown.contains("Org org-a"),
        "own terminal (reveal): {shown}"
    );

    // Under Claude Code, even an agent's own SEATWISE_REVEAL doesn't show them.
    let t = ok(&e
        .seat("", &["list"])
        .env("CLAUDECODE", "1")
        .output()
        .unwrap());
    assert!(!t.contains("org-a") && t.contains("<org-name:a>"), "{t}");
    // Piped, without reveal: hidden too.
    let t = text(
        &e.seat("", &["doctor"])
            .env_remove("SEATWISE_REVEAL")
            .output()
            .unwrap(),
    );
    assert!(!t.contains("org-a") && !t.contains("org-x"), "{t}");

    // Before there is a config, the accounts found on the machine are hidden too.
    fs::create_dir_all(e.home.join(".claude-x/projects")).unwrap();
    fs::write(
        e.home.join(".claude-x/.claude.json"),
        r#"{"oauthAccount":{"organizationUuid":"0b9c7a52-1e4f-4f0e-9a51-6f4f0b0d2c11","organizationName":"Secret Org","emailAddress":"x@secret.example"}}"#,
    )
    .unwrap();
    let t = ok(&e
        .seat("", &["init", "--print"])
        .env("CLAUDECODE", "1")
        .env("SEATWISE_CONFIG", e.home.join("none.toml"))
        .output()
        .unwrap());
    assert!(t.contains("[[seat]]") && t.contains("<org-name:x>"), "{t}");
    assert!(
        !t.contains("Secret") && !t.contains("secret.example") && !t.contains("0b9c7a52"),
        "{t}"
    );

    // Hook messages reach the model: no identifiers in them either.
    use std::io::Write;
    let mut child = e
        .seat("", &["hook", "prompt", "b"])
        .env("CLAUDECODE", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = serde_json::json!({ "cwd": p(&e.home.join("proj-b")), "prompt": "hi" });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "{t}");
    assert!(
        t.contains("another org") && !t.contains("org-x") && !t.contains("org-b"),
        "{t}"
    );
}

#[test]
fn errors_and_usage_messages_are_redacted_in_process() {
    let e = setup();
    let id = "12345678-90ab-4cde-8f01-234567890abc";
    // An error that echoes what was typed.
    let t = text(
        &e.seat("", &["show", id])
            .env("CLAUDECODE", "1")
            .env_remove("SEATWISE_REVEAL")
            .output()
            .unwrap(),
    );
    assert!(
        t.contains("unknown seat '<uuid>'") && !t.contains(id),
        "{t}"
    );
    // clap's own usage error, printed by clap when nothing is redacted and through the redactor when it is.
    let t = text(
        &e.seat("", &["--no-such-flag", id])
            .env("CLAUDECODE", "1")
            .env_remove("SEATWISE_REVEAL")
            .output()
            .unwrap(),
    );
    assert!(t.contains("unexpected argument") && !t.contains(id), "{t}");
    // Output that isn't redacted stays whole: shell code and completions are never rewritten.
    let t = ok(&e
        .seat("", &["completions", "bash"])
        .env("CLAUDECODE", "1")
        .output()
        .unwrap());
    assert!(t.contains("complete"), "{t}");
}

#[test]
fn the_hybrid_plan_lists_only_open_choices_and_valid_session_settings() {
    let e = setup_with("[profile.broken]\npermissions = { defaultMode = \"sometimes\" }\n");
    let t = ok(&e
        .seat(
            "",
            &["hybrid", "--json", "--seat", "a", "--dir", "elsewhere"],
        )
        .output()
        .unwrap());
    let plan: serde_json::Value = serde_json::from_str(&t).expect(&t);
    let steps: Vec<&str> = plan["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["step"].as_str().unwrap())
        .collect();
    assert_eq!(steps, ["owner", "preset", "session"], "{t}");
    let session = &plan["choices"][2]["options"];
    let experimental = session
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["value"] == "experimental")
        .expect("the experimental profile is offered");
    assert_eq!(experimental["flag"], "--with experimental");
    assert_eq!(experimental["experimental"], true);
    assert!(
        !session.to_string().contains("broken"),
        "a profile Claude Code would reject is not offered: {t}"
    );
    assert!(
        !read(&e.config).contains("elsewhere"),
        "--json changes nothing"
    );

    // Chosen: nothing left open for those steps.
    let t = ok(&e
        .seat(
            "",
            &[
                "hybrid",
                "--json",
                "--seat",
                "a",
                "--dir",
                "elsewhere",
                "--link",
                "--preset",
                "build",
                "--with",
                "experimental",
            ],
        )
        .output()
        .unwrap());
    let plan: serde_json::Value = serde_json::from_str(&t).expect(&t);
    assert_eq!(plan["choices"], serde_json::json!([]), "{t}");
    assert!(
        plan["terminal"].as_str().unwrap().contains("experimental"),
        "{t}"
    );

    // Asked for by name, a bad profile is refused before anything changes.
    let out = e
        .seat(
            "",
            &[
                "hybrid",
                "--seat",
                "a",
                "--dir",
                "elsewhere",
                "--link",
                "--with",
                "broken",
            ],
        )
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(text(&out).contains("defaultMode"), "{}", text(&out));
    assert!(!read(&e.config).contains("elsewhere"));
}

#[test]
fn hybrid_checks_then_sets_up_the_project_and_the_terminal() {
    let e = setup_with("");
    let t = ok(&e
        .seat(
            "",
            &["hybrid", "--check", "--seat", "a", "--dir", "elsewhere"],
        )
        .output()
        .unwrap());
    assert!(
        t.contains("not assigned") && t.contains("skills   missing"),
        "{t}"
    );
    assert!(
        !e.home.join(".seat-a/seatwise-plugins").exists(),
        "--check changes nothing"
    );
    assert!(!read(&e.config).contains("elsewhere"));

    let t = ok(&e
        .seat(
            "",
            &[
                "hybrid",
                "--seat",
                "a",
                "--dir",
                "elsewhere",
                "--link",
                "--preset",
                "explore",
                "--with",
                "light",
                "--print",
            ],
        )
        .output()
        .unwrap());
    assert!(
        t.contains("now belongs to a") && t.contains("preset   explore (group 'elsewhere')"),
        "{t}"
    );
    assert!(t.contains("light"), "{t}");
    assert!(
        e.home
            .join(".seat-a/seatwise-plugins/plugins/seatwise/skills/hybrid/SKILL.md")
            .is_file()
    );
    let t = ok(&e
        .seat(
            "",
            &["hybrid", "--check", "--seat", "a", "--dir", "elsewhere"],
        )
        .output()
        .unwrap());
    assert!(
        t.contains("owner    a: ok") && t.contains("skills   up to date"),
        "{t}"
    );
    assert!(t.contains("preset   explore from group 'elsewhere'"), "{t}");

    // From a session of seat b in its project; never into another seat's folder.
    let t = ok(&e
        .seat("", &["hybrid", "--check"])
        .env("CLAUDE_CONFIG_DIR", e.home.join(".seat-b"))
        .env("CLAUDE_PROJECT_DIR", e.home.join("proj-b"))
        .output()
        .unwrap());
    assert!(t.contains("seat     b"), "{t}");
    let out = e
        .seat("", &["hybrid", "--seat", "a", "--dir", "proj-b", "--print"])
        .output()
        .unwrap();
    assert!(
        !out.status.success() && text(&out).contains("belongs to seat 'b'"),
        "{}",
        text(&out)
    );
}

#[test]
fn json_for_orchestrators() {
    let e = setup();
    let json = |dir: &str, args: &[&str]| -> serde_json::Value {
        let out = e.seat(dir, args).output().unwrap();
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|err| panic!("{args:?}: {err}: {}", text(&out)))
    };
    let v = json("proj-a", &["owner", "--json"]);
    assert_eq!(v["schema"], 1);
    assert_eq!(
        (v["kind"].as_str(), v["owner"].as_str()),
        (Some("seat"), Some("a"))
    );
    assert_eq!(v["run_seat"], "a");
    assert_eq!(v["enforcement"], "strict");
    let v = json("", &["owner", "--json", "/nowhere"]);
    assert_eq!(v["kind"], "unassigned");
    assert!(v["run_seat"].is_null(), "strict: refused, so no seat: {v}");

    let v = json("", &["list", "--json"]);
    let seats = v["seats"].as_array().unwrap();
    assert_eq!(seats[0]["name"], "a");
    assert!(seats[0]["folders"].as_array().is_some());
    assert!(!v.to_string().contains("org-a"), "no identifiers: {v}");

    let v = json("", &["ps", "--json"]);
    assert!(v["sessions"].is_array(), "{v}");

    let v = json("proj-a", &["explain", "--json"]);
    assert_eq!(v["seat"], "a");
    assert!(
        v["layers"].is_array() && v["fingerprint"].is_string(),
        "{v}"
    );

    let out = e.seat("", &["doctor", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["hook_contract"], 1);
    assert_eq!(v["ok"], out.status.success(), "exit code and ok agree: {v}");
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["level"].is_string())
    );
}

#[test]
fn config_from_a_newer_seatwise_is_refused() {
    let e = setup();
    let text_now = fs::read_to_string(&e.config).unwrap();
    fs::write(&e.config, format!("version = 1\n{text_now}")).unwrap();
    assert!(e.seat("", &["list"]).output().unwrap().status.success());
    fs::write(&e.config, format!("version = 99\n{text_now}")).unwrap();
    let out = e.seat("", &["list"]).output().unwrap();
    let t = text(&out);
    assert!(!out.status.success(), "{t}");
    assert!(
        t.contains("format version 99") && t.contains("resolution: Upgrade seatwise"),
        "{t}"
    );
}
