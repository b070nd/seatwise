//! Host compatibility: seatwise against the real, current Claude Code and Codex CLIs (not the fake ones the other
//! tests use). Ignored by default; the `host-compat` workflow runs them weekly against the latest releases:
//!
//!   SEATWISE_HOST_CLAUDE=$(which claude) SEATWISE_HOST_CODEX=$(which codex) SEATWISE_HOST_OUT=out \
//!     cargo test --test host -- --ignored
//!
//! No account is involved, so what they can check is the part of the contract that needs none: the agent starts on a
//! guarded seat with seatwise's environment and its `--version` probe, and the hooks seatwise writes are left where
//! the agent reads them (the workflow then validates Claude Code's settings.json against its published schema).
//! Whether a signed-in session fires the hooks is checked by hand before a release (see CONTRIBUTING.md).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn host(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// A temp home with one strict seat for `agent`, owning `~/proj`.
fn setup(agent: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let home = dunce::canonicalize(tmp.path()).unwrap();
    fs::create_dir_all(home.join("proj")).unwrap();
    let config = home.join("config.toml");
    fs::write(
        &config,
        format!(
            "version = 1\nenforcement = \"strict\"\n[[seat]]\nname = \"s\"\nconfig_dir = \"~/.seat-s\"\nagent = \
             \"{agent}\"\n[[folder]]\npath = \"~/proj\"\nowner = \"s\"\n"
        ),
    )
    .unwrap();
    (tmp, home, config)
}

fn seat(home: &Path, config: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_seat"));
    cmd.args(args)
        .current_dir(home.join("proj"))
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("SEATWISE_HOME", home)
        .env("SEATWISE_CONFIG", config)
        .env("SEATWISE_DATA", home.join("data"))
        .env("SEATWISE_NO_KEYCHAIN", "1")
        .env("SEATWISE_REVEAL", "1")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env_remove("SEATWISE_SEAT")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME");
    cmd
}

fn check(agent: &str, var: &str, override_var: &str, hooks_file: &str) {
    let Some(bin) = host(var) else {
        panic!("set {var} to the real {agent} binary");
    };
    let (_tmp, home, config) = setup(if agent == "claude" { "claude" } else { "codex" });
    let out = seat(&home, &config, &["run", "s", "--version"])
        .env(override_var, &bin)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{agent} --version on a strict seat: {text}"
    );
    assert!(
        text.chars().any(|c| c.is_ascii_digit()),
        "a version: {text}"
    );
    let hooks = home.join(".seat-s").join(hooks_file);
    let written = fs::read_to_string(&hooks).unwrap_or_else(|e| panic!("{}: {e}", hooks.display()));
    assert!(written.contains("hook tool s --contract"), "{written}");
    let doctor = seat(&home, &config, &["doctor", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert!(
        !v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["level"] == "error"
                && f["message"].as_str().is_some_and(|m| m.contains("hook"))),
        "doctor finds the hooks current: {v}"
    );
    if let Some(out_dir) = host("SEATWISE_HOST_OUT") {
        fs::create_dir_all(&out_dir).unwrap();
        fs::copy(&hooks, out_dir.join(format!("{agent}-{hooks_file}"))).unwrap();
    }
}

#[test]
#[ignore = "needs the real Claude Code CLI (SEATWISE_HOST_CLAUDE)"]
fn claude_code_starts_on_a_guarded_seat() {
    check(
        "claude",
        "SEATWISE_HOST_CLAUDE",
        "SEATWISE_CLAUDE",
        "settings.json",
    );
}

#[test]
#[ignore = "needs the real Codex CLI (SEATWISE_HOST_CODEX)"]
fn codex_starts_on_a_guarded_seat() {
    check(
        "codex",
        "SEATWISE_HOST_CODEX",
        "SEATWISE_CODEX",
        "hooks.json",
    );
}
