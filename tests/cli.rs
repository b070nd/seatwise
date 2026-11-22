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
fn wrong_org_is_refused_unless_forced() {
    let e = setup();
    let t = text(&e.seat("proj-b", &["run", "auto"]).output().unwrap());
    assert!(t.contains("wrong org") && !t.contains("CFG="), "{t}");
    let t = text(
        &e.seat("proj-b", &["run", "auto"])
            .env("SEATWISE_FORCE", "1")
            .output()
            .unwrap(),
    );
    assert!(t.contains("SEAT=b"), "{t}");
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
        t.contains("warning") && t.contains("wrong org") && t.contains("SEAT=b"),
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
    assert!(t.contains("wrong org"), "{t}");
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
    assert!(cfg.contains("path = \"~/proj-b\"\nowner = \"y\""), "{cfg}");
    assert!(!cfg.contains("nested"), "covered by proj-a: {cfg}");
    // An existing config is never overwritten without --force.
    let out = e
        .seat("", &["init"])
        .env("SEATWISE_CONFIG", &target)
        .output()
        .unwrap();
    assert!(text(&out).contains("already exists"), "{}", text(&out));
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
        !e.home.join(".seat-a/settings.json").exists(),
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
    let skill = read(&e.home.join(".seat-a/skills/seat-terminal/SKILL.md"));
    assert!(skill.starts_with("---\nname: seat-terminal"), "{skill}");
    assert!(
        !skill.contains("{{seat}}") && skill.contains(env!("CARGO_BIN_EXE_seat")),
        "{skill}"
    );
    assert!(
        !e.home.join(".seat-b/skills").exists(),
        "only the seat asked for"
    );
    let t = text(
        &e.seat("", &["skills", "show", "seat-home"])
            .output()
            .unwrap(),
    );
    assert!(t.contains("name: seat-home"), "{t}");
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
        !sh.contains("claude-a()") && sh.contains("claude-b()"),
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
