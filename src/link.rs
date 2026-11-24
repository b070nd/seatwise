//! Where a seat's links go. When a seat's `link` is not "system", the launcher sets `BROWSER` to the `seat`
//! binary, so Claude Code calls `seat <url>` and the seat's setting decides: copy, or run a command
//! (e.g. a browser with a profile flag).

use crate::config::{Config, Link, Seat};
use crate::errln;
use crate::folders::normalize;
use crate::paths;
use anyhow::{Context, Result, bail};
use std::process::{Command, Stdio};

/// The seat a link belongs to: `SEATWISE_SEAT` (set by the launcher), else the seat whose config folder is
/// `CLAUDE_CONFIG_DIR`.
pub fn current_seat(cfg: &Config) -> Option<&Seat> {
    if let Ok(name) = std::env::var("SEATWISE_SEAT")
        && let Some(seat) = cfg.seat(&name)
    {
        return Some(seat);
    }
    let home = paths::home();
    for (var, default) in [("CLAUDE_CONFIG_DIR", ".claude"), ("CODEX_HOME", ".codex")] {
        let dir = std::env::var_os(var)
            .map(|d| normalize(std::path::Path::new(&d), &home))
            .unwrap_or_else(|| normalize(&home.join(default), &home));
        if std::env::var_os(var).is_some()
            && let Some(s) = cfg
                .seats
                .iter()
                .find(|s| normalize(&s.config_dir(), &home) == dir)
        {
            return Some(s);
        }
    }
    // The desktop app exports the org it is signed into: the seat pinned to it, if exactly one is.
    if let Ok(org) = std::env::var("CLAUDE_CODE_ORGANIZATION_UUID") {
        let pinned: Vec<&Seat> = cfg
            .seats
            .iter()
            .filter(|s| s.org_uuid.as_deref() == Some(org.as_str()))
            .collect();
        if let [only] = pinned.as_slice() {
            return Some(only);
        }
    }
    // Claude Code's default location.
    let dir = normalize(&home.join(".claude"), &home);
    cfg.seats
        .iter()
        .find(|s| normalize(&s.config_dir(), &home) == dir)
}

/// Handle a link for the current seat. Without a readable config or a known seat, copy it: never lose a link.
pub fn open(url: &str) -> Result<()> {
    let cfg = Config::load().ok();
    let seat = cfg.as_ref().and_then(current_seat);
    let label = seat.map(|s| s.name.as_str());
    match seat.map_or(Link::Copy, Seat::link) {
        Link::Copy => copy(url, label),
        Link::System => system_open(url),
        Link::Command(argv) => run_command(argv, url).or_else(|e| {
            errln!("seat: {e:#}");
            copy(url, label)
        }),
    }
}

/// Copy the link and say so.
pub fn copy(url: &str, seat: Option<&str>) -> Result<()> {
    let which = seat
        .map(|s| format!("seat '{s}'"))
        .unwrap_or_else(|| "this seat".into());
    if let Err(e) = set_clipboard(url) {
        // Never lose the link: show it so it can be copied by hand.
        errln!("seat: could not copy the link ({e}). Open it in the browser for {which}:\n{url}");
        return Ok(());
    }
    let _ = notify_rust::Notification::new()
        .summary("Link copied")
        .body(&format!("Paste it into the browser for {which}."))
        .show();
    errln!("seat: link copied to the clipboard ({which})");
    Ok(())
}

/// `argv` with `{url}` replaced by the link (or the link appended).
pub fn command_line(argv: &[String], url: &str) -> Vec<String> {
    let mut out: Vec<String> = argv.iter().map(|a| a.replace("{url}", url)).collect();
    if !argv.iter().any(|a| a.contains("{url}")) {
        out.push(url.to_string());
    }
    out
}

fn run_command(argv: &[String], url: &str) -> Result<()> {
    let line = command_line(argv, url);
    // Not waited for: a browser started this way may keep running.
    Command::new(paths::expand(&line[0]))
        .args(&line[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map(drop)
        .with_context(|| format!("could not run link command '{}'", argv[0]))
}

/// The operating system's default browser.
fn system_open(url: &str) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    // xdg-open may fall back to $BROWSER, which could be us.
    cmd.arg(url).env_remove("BROWSER");
    spawn(&mut cmd).context("could not open the default browser")
}

fn spawn(cmd: &mut Command) -> Result<()> {
    let status = cmd.stdin(Stdio::null()).stdout(Stdio::null()).status()?;
    if !status.success() {
        bail!("exited with {status}");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn set_clipboard(text: &str) -> Result<()> {
    arboard::Clipboard::new()?.set_text(text)?;
    Ok(())
}

/// X11 and Wayland clipboards vanish with the process that owns them, so hand the text to a tool that stays around.
#[cfg(target_os = "linux")]
fn set_clipboard(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let tools: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (prog, args) in tools {
        let Ok(mut child) = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(text.as_bytes())?;
        if child.wait()?.success() {
            return Ok(());
        }
    }
    anyhow::bail!("no working clipboard tool: install wl-clipboard (Wayland) or xclip (X11)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_command_line() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            command_line(&argv(&["firefox", "-P", "work", "{url}"]), "https://x"),
            argv(&["firefox", "-P", "work", "https://x"])
        );
        assert_eq!(
            command_line(
                &argv(&["chrome", "--profile-directory=Profile 2"]),
                "https://x"
            ),
            argv(&["chrome", "--profile-directory=Profile 2", "https://x"]),
            "appended when there is no placeholder"
        );
    }
}
