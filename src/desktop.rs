//! `seat desktop <seat>`: the Claude desktop app on a seat (macOS).
//!
//! Every seat runs the same installed app, with its own data folder (`--user-data-dir`, the desktop's own login and
//! settings) and the seat's environment (`CLAUDE_CONFIG_DIR`, `SEATWISE_SEAT`, `BROWSER`), so the Claude Code
//! sessions it starts are on that seat. No copies of the app are needed.
//!
//! What the app doesn't do for us, and this module does:
//! - one instance per data folder: `open -n` happily starts a second one on the same folder;
//! - a warning before a first sign-in while other instances run: macOS hands the Google sign-in answer (a
//!   `claude://` link) to the oldest running instance, which ignores it, so the new one never signs in;
//! - finding instances started without a seat (from the Dock or Finder) or with an outdated environment.

use crate::config::{Config, Link, Seat};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const DEFAULT_APP: &str = "/Applications/Claude.app";

fn app_support() -> PathBuf {
    crate::paths::home().join("Library/Application Support")
}

/// The app bundle a seat runs.
pub fn app(seat: &Seat) -> PathBuf {
    seat.desktop
        .as_ref()
        .and_then(|d| d.app.as_deref())
        .map(crate::paths::expand)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_APP))
}

/// The seat's data folder; `None` is the app's default folder (no `--user-data-dir`).
pub fn data_dir(seat: &Seat) -> Option<PathBuf> {
    match seat
        .desktop
        .as_ref()
        .and_then(|d| d.user_data_dir.as_deref())
    {
        Some("default") => None,
        Some(dir) => Some(crate::paths::expand(dir)),
        None => Some(app_support().join(format!("Claude-{}", seat.name))),
    }
}

/// The folder the app actually uses for a seat.
fn data_dir_or_default(seat: &Seat) -> PathBuf {
    data_dir(seat).unwrap_or_else(|| app_support().join("Claude"))
}

/// The environment the app gets for a seat. The app inherits the login session's environment from launchd, never
/// the shell's, so this is all it gets from us.
pub fn env_for(seat: &Seat, exe: &Path) -> Vec<(&'static str, String)> {
    let mut env = Vec::new();
    if !seat.is_default_location() {
        env.push(("CLAUDE_CONFIG_DIR", seat.config_dir().display().to_string()));
    }
    env.push(("SEATWISE_SEAT", seat.name.clone()));
    if seat.link() != Link::System {
        env.push(("BROWSER", exe.display().to_string()));
    }
    env
}

/// A running desktop instance (its main process).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub pid: u32,
    pub started: String,
    pub app: PathBuf,
    /// `None`: the app's default data folder.
    pub data_dir: Option<PathBuf>,
    pub seat: Option<String>,
    pub config_dir: Option<String>,
    pub browser: Option<String>,
}

impl Instance {
    fn data_dir_or_default(&self) -> PathBuf {
        self.data_dir
            .clone()
            .unwrap_or_else(|| app_support().join("Claude"))
    }

    pub fn label(&self) -> String {
        format!(
            "pid {}, started {}, data {}",
            self.pid,
            self.started,
            crate::paths::contract(&self.data_dir_or_default())
        )
    }
}

/// One process as `ps` shows it: pid, start time, command line, environment (as one string).
#[derive(Debug, Clone)]
struct Raw {
    pid: u32,
    started: String,
    command: String,
    env: String,
}

/// The value of `--user-data-dir=` in a command line (it may contain spaces: it ends at the next ` --`).
fn user_data_dir_arg(command: &str) -> Option<PathBuf> {
    let start = command.find("--user-data-dir=")? + "--user-data-dir=".len();
    let rest = &command[start..];
    let end = rest.find(" --").unwrap_or(rest.len());
    let value = rest[..end].trim().trim_matches('"');
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// The value of `KEY=` in `ps eww` output, where variables are separated by spaces. A value ends where the next
/// ` NAME=` starts, so values may contain spaces.
fn env_value(env: &str, key: &str) -> Option<String> {
    let padded = format!(" {env}");
    let needle = format!(" {key}=");
    let start = padded.find(&needle)? + needle.len();
    let rest = &padded[start..];
    let mut end = rest.len();
    for (i, _) in rest.match_indices(' ') {
        let name: String = rest[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && rest[i + 1 + name.len()..].starts_with('=')
        {
            end = i;
            break;
        }
    }
    Some(rest[..end].to_string())
}

/// The app bundles seats run: the default one and any set in `[seat.desktop]`.
fn apps(cfg: &Config) -> Vec<PathBuf> {
    let mut apps = vec![PathBuf::from(DEFAULT_APP)];
    for seat in &cfg.seats {
        let a = app(seat);
        if !apps.contains(&a) {
            apps.push(a);
        }
    }
    apps
}

/// Which of `apps` a process is the main process of: its executable sits directly in `<app>/Contents/MacOS/`.
/// Helpers live in `Contents/Frameworks`, and Claude Code sessions run a `claude.app` of their own inside the data
/// folder, so neither matches.
fn main_process_app(command: &str, apps: &[PathBuf]) -> Option<PathBuf> {
    apps.iter()
        .find(|a| {
            let prefix = format!("{}/Contents/MacOS/", a.display());
            command.starts_with(&prefix)
                && !command[prefix.len()..]
                    .split(" --")
                    .next()
                    .unwrap_or_default()
                    .contains('/')
        })
        .cloned()
}

fn parse(raw: &[Raw], apps: &[PathBuf]) -> Vec<Instance> {
    raw.iter()
        .filter_map(|r| {
            let app = main_process_app(&r.command, apps)?;
            Some(Instance {
                pid: r.pid,
                started: r.started.clone(),
                app,
                data_dir: user_data_dir_arg(&r.command),
                seat: env_value(&r.env, "SEATWISE_SEAT"),
                config_dir: env_value(&r.env, "CLAUDE_CONFIG_DIR"),
                browser: env_value(&r.env, "BROWSER"),
            })
        })
        .collect()
}

/// `ps` output, or a file standing in for it (`SEATWISE_DESKTOP_PS`: per line, tab-separated pid, start time,
/// command, environment; used by the tests).
fn raw_processes(apps: &[PathBuf]) -> Vec<Raw> {
    if let Some(file) = std::env::var_os("SEATWISE_DESKTOP_PS") {
        return std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(4, '\t');
                Some(Raw {
                    pid: f.next()?.trim().parse().ok()?,
                    started: f.next()?.to_string(),
                    command: f.next()?.to_string(),
                    env: f.next().unwrap_or_default().to_string(),
                })
            })
            .collect();
    }
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let Ok(out) = std::process::Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,lstart=,command="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (pid, rest) = l.trim_start().split_once(' ')?;
            // `lstart` is five words ("Sun Nov  2 22:43:01 2026", single-digit days padded), then the command.
            let (words, command) = after_words(rest, 5)?;
            let started = words.join(" ");
            let command = command.to_string();
            main_process_app(&command, apps)?;
            // The environment of our own processes: `ps eww` prints it after the command.
            let env = std::process::Command::new("/bin/ps")
                .args(["eww", "-o", "command=", "-p", pid])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .and_then(|full| full.strip_prefix(command.trim()).map(str::to_string))
                .unwrap_or_default();
            Some(Raw {
                pid: pid.parse().ok()?,
                started,
                command,
                env,
            })
        })
        .collect()
}

/// The first `n` whitespace-separated words of `s`, and the rest after them.
fn after_words(s: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut words = Vec::new();
    let mut rest = s;
    for _ in 0..n {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        words.push(&rest[..end]);
        rest = &rest[end..];
    }
    Some((words, rest.trim_start()))
}

/// The desktop instances running now.
pub fn running(cfg: &Config) -> Vec<Instance> {
    let apps = apps(cfg);
    parse(&raw_processes(&apps), &apps)
}

/// The desktop instances of the default app running now, before there is a config (`seat init`).
pub fn running_default_app() -> Vec<Instance> {
    let apps = [PathBuf::from(DEFAULT_APP)];
    parse(&raw_processes(&apps), &apps)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    crate::folders::key_string(a) == crate::folders::key_string(b)
}

/// The Claude Code config folder an instance's sessions use (`~/.claude` without `CLAUDE_CONFIG_DIR`).
fn instance_config_dir(i: &Instance) -> PathBuf {
    i.config_dir
        .as_deref()
        .map(crate::paths::expand)
        .unwrap_or_else(|| crate::paths::home().join(".claude"))
}

/// An instance in the app's default data folder whose sessions use `config_dir` (started from the Dock for
/// `~/.claude`, or with `CLAUDE_CONFIG_DIR` set by hand or by a setup from before seatwise): that folder holds the
/// desktop login of the seat living in `config_dir`.
pub fn on_default_data_dir(instances: &[Instance], config_dir: &Path) -> bool {
    instances
        .iter()
        .any(|i| i.data_dir.is_none() && same_dir(&instance_config_dir(i), config_dir))
}

/// Whether instance `i` runs in the app's default data folder for seat `seat`: started for it by name, with its
/// config folder set by hand, or from the Dock for a seat living in `~/.claude`.
fn runs_default_for(cfg: &Config, i: &Instance, seat: &Seat) -> bool {
    i.data_dir.is_none()
        && match i.seat.as_deref() {
            Some(name) => cfg.seat(name).is_some_and(|x| x.name == seat.name),
            None => same_dir(&instance_config_dir(i), &seat.config_dir()),
        }
}

/// The seat whose desktop app runs in the app's default data folder while its config gives it a folder of its own:
/// `user_data_dir = "default"` keeps that folder's desktop login. `None` when a seat already claims the default
/// folder, or more than one seat would.
pub fn default_folder_seat(cfg: &Config, running: &[Instance]) -> Option<String> {
    let claude = || {
        cfg.seats
            .iter()
            .filter(|s| s.agent == crate::config::Agent::Claude)
    };
    if claude().any(|s| data_dir(s).is_none()) {
        return None;
    }
    let explicit = |s: &Seat| {
        s.desktop
            .as_ref()
            .is_some_and(|d| d.user_data_dir.is_some())
    };
    let mut seats = claude()
        .filter(|s| !explicit(s))
        .filter(|s| running.iter().any(|i| runs_default_for(cfg, i, s)));
    let seat = seats.next()?;
    seats.next().is_none().then(|| seat.name.clone())
}

fn default_folder_hint(name: &str) -> String {
    format!(
        "add `user_data_dir = \"default\"` under [seat.desktop] for seat '{name}' to keep that login \
         (`seat doctor --fix` adds it)"
    )
}

/// Whether instance `i` is the one that makes `adopt` the default-folder seat.
fn adopted(cfg: &Config, i: &Instance, adopt: Option<&str>) -> bool {
    adopt
        .and_then(|name| cfg.seat(name))
        .is_some_and(|s| runs_default_for(cfg, i, s))
}

/// Start a seat's desktop app, unless it already runs. Returns what happened, one line per point.
pub fn open(cfg: &Config, seat: &Seat, exe: &Path) -> Result<Vec<String>> {
    if seat.agent != crate::config::Agent::Claude {
        bail!(
            "seat '{}' runs Codex, which has no desktop app here",
            seat.name
        );
    }
    if !cfg.is_enabled(seat) {
        bail!(
            "seat '{}' is disabled (`seat enable {}`)",
            seat.name,
            seat.name
        );
    }
    let opener = std::env::var_os("SEATWISE_OPEN").map(PathBuf::from);
    if opener.is_none() && !cfg!(target_os = "macos") {
        bail!("`seat desktop` supports macOS only for now");
    }
    let app = app(seat);
    if opener.is_none() && !app.is_dir() {
        bail!(
            "{} doesn't exist; set `app` in [seat.desktop] for seat '{}'",
            app.display(),
            seat.name
        );
    }
    let data = data_dir_or_default(seat);
    let running = running(cfg);
    if default_folder_seat(cfg, &running).as_deref() == Some(seat.name.as_str()) {
        return Ok(vec![format!(
            "seat '{}' desktop already runs in the app's default data folder; not starting a second one with a \
             new, signed-out folder. {}",
            seat.name,
            default_folder_hint(&seat.name)
        )]);
    }
    if let Some(i) = running
        .iter()
        .find(|i| same_dir(&i.data_dir_or_default(), &data))
    {
        let mut out = vec![format!(
            "seat '{}' desktop is already running ({}); not starting a second instance on the same data folder",
            seat.name,
            i.label()
        )];
        if let Some(diff) = stale(cfg, i, exe) {
            out.push(format!(
                "it runs with an older setup ({diff}): quit it (Cmd-Q in that window) and run this again"
            ));
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    if !data.exists() && !running.is_empty() {
        out.push(format!(
            "note: seat '{}' has no desktop data folder yet, so it will ask you to sign in while {} other Claude \
             window(s) run. A Google sign-in answers through a claude:// link, which macOS hands to the oldest \
             running Claude, so it never arrives. Sign in with your email (you type the code into the app), or quit \
             the other Claude windows until this one is signed in.",
            seat.name,
            running.len()
        ));
    }
    let mut cmd =
        std::process::Command::new(opener.unwrap_or_else(|| PathBuf::from("/usr/bin/open")));
    cmd.arg("-n").arg("-a").arg(&app);
    for (k, v) in env_for(seat, exe) {
        cmd.arg("--env").arg(format!("{k}={v}"));
    }
    if let Some(dir) = data_dir(seat) {
        cmd.arg("--args")
            .arg(format!("--user-data-dir={}", dir.display()));
    }
    let output = cmd.output().context("starting the desktop app")?;
    if !output.status.success() {
        bail!(
            "open failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    // `open` prints nothing; the tests' stand-in prints what it was given.
    out.extend(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string),
    );
    out.push(format!(
        "started the desktop app on seat '{}' (data {})",
        seat.name,
        crate::paths::contract(&data)
    ));
    Ok(out)
}

/// How a running instance's setup differs from what `seat desktop` would start now, if it does.
fn stale(cfg: &Config, i: &Instance, exe: &Path) -> Option<String> {
    let seat = cfg.seat(i.seat.as_deref()?)?;
    let want = env_for(seat, exe);
    let get = |k: &str| want.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
    // Paths are compared after resolving symlinks: `~/.local/bin/seat` linking to `~/.cargo/bin/seat` is the same.
    let same = |have: &Option<String>, want: Option<String>| match (have, want) {
        (None, None) => true,
        (Some(h), Some(w)) => {
            let real = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
            h == &w || same_dir(&real(h), &real(&w))
        }
        _ => false,
    };
    let mut diffs = Vec::new();
    if !same(&i.config_dir, get("CLAUDE_CONFIG_DIR")) {
        diffs.push("CLAUDE_CONFIG_DIR");
    }
    if !same(&i.browser, get("BROWSER")) {
        diffs.push("BROWSER");
    }
    if !same_dir(&i.data_dir_or_default(), &data_dir_or_default(seat)) {
        diffs.push("data folder");
    }
    (!diffs.is_empty()).then(|| format!("{} differ", diffs.join(", ")))
}

/// Problems with the running instances, for `seat doctor`.
pub struct Findings {
    pub warnings: Vec<String>,
    /// A seat that should get `user_data_dir = "default"` (see [`default_folder_seat`]); its warning is the first.
    pub default_folder_seat: Option<String>,
}

pub fn findings(cfg: &Config, exe: &Path) -> Findings {
    let running = running(cfg);
    let adopt = default_folder_seat(cfg, &running);
    let mut out = Vec::new();
    if let Some(name) = &adopt {
        out.push(format!(
            "seat '{name}' desktop runs in the app's default data folder, where its desktop login is, but its \
             config gives it a folder of its own: {}",
            default_folder_hint(name)
        ));
    }
    for (n, i) in running.iter().enumerate() {
        if running[..n]
            .iter()
            .any(|j| same_dir(&j.data_dir_or_default(), &i.data_dir_or_default()))
        {
            out.push(format!(
                "two Claude desktop instances share one data folder ({}): quit one; `seat desktop` never starts a second",
                i.label()
            ));
        }
        if adopted(cfg, i, adopt.as_deref()) {
            continue;
        }
        match i.seat.as_deref() {
            None if i.config_dir.is_none() => out.push(format!(
                "a Claude desktop instance runs without a seat ({}): it was started from the Dock or Finder, so its \
                 Code sessions use ~/.claude, without hooks or a pinned org. Quit it and start it with `seat desktop <seat>`",
                i.label()
            )),
            None => {}
            Some(name) if cfg.seat(name).is_none() => out.push(format!(
                "a Claude desktop instance runs for seat '{name}', which isn't configured ({})",
                i.label()
            )),
            Some(name) => {
                if let Some(diff) = stale(cfg, i, exe) {
                    out.push(format!(
                        "restart the '{name}' desktop: it runs with an older setup ({diff}; {}). Quit it and run `seat desktop {name}`",
                        i.label()
                    ));
                }
            }
        }
    }
    Findings {
        warnings: out,
        default_folder_seat: adopt,
    }
}

/// `seat desktop` without a seat: the running instances.
pub fn status(cfg: &Config, exe: &Path) -> Vec<String> {
    let running = running(cfg);
    if running.is_empty() {
        return vec!["no Claude desktop instance is running".into()];
    }
    let adopt = default_folder_seat(cfg, &running);
    let mut out: Vec<String> = running
        .iter()
        .map(|i| {
            let mine = adopted(cfg, i, adopt.as_deref());
            let who = match (&i.seat, &i.config_dir) {
                (Some(s), _) => format!("seat {s}"),
                (None, Some(_)) if mine => format!("seat {}", adopt.as_deref().unwrap_or_default()),
                (None, Some(_)) => "no seat (CLAUDE_CONFIG_DIR set by hand)".into(),
                (None, None) => "no seat".into(),
            };
            let note = if mine {
                format!(
                    "  [{}]",
                    default_folder_hint(adopt.as_deref().unwrap_or_default())
                )
            } else {
                stale(cfg, i, exe)
                    .map(|d| format!("  [restart: {d}]"))
                    .unwrap_or_default()
            };
            format!("{who:<14} {}{note}", i.label())
        })
        .collect();
    out.push(format!(
        "every instance writes to {}; match its lines to an instance by the start times above",
        crate::paths::contract(&crate::paths::home().join("Library/Logs/Claude/main.log"))
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_data_folders_and_environment_from_ps() {
        let raw = vec![
            Raw {
                pid: 10,
                started: "Sun Nov 22 22:43:01 2026".into(),
                command: "/Applications/Claude.app/Contents/MacOS/Claude --user-data-dir=/Users/u/Library/Application Support/Claude-frontier --foo".into(),
                env: "PATH=/usr/bin SEATWISE_SEAT=frontier CLAUDE_CONFIG_DIR=/Users/u/my configs/.claude-frontier BROWSER=/bin/seat HOME=/Users/u".into(),
            },
            Raw {
                pid: 11,
                started: "x".into(),
                command: "/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper --type=renderer".into(),
                env: String::new(),
            },
            Raw {
                pid: 12,
                started: "y".into(),
                command: "/Applications/Claude.app/Contents/MacOS/Claude".into(),
                env: "PATH=/usr/bin".into(),
            },
        ];
        let raw = [
            raw,
            vec![
                Raw {
                    pid: 13,
                    started: "z".into(),
                    command: "/Users/u/Library/Application Support/Claude/claude-code/2.1/h/claude.app/Contents/MacOS/claude --output-format stream-json".into(),
                    env: "SEATWISE_SEAT=work".into(),
                },
                Raw {
                    pid: 14,
                    started: "z".into(),
                    command: "/Users/u/Applications/Claude Link Router.app/Contents/MacOS/claude-link-router".into(),
                    env: String::new(),
                },
            ],
        ]
        .concat();
        let got = parse(&raw, &[PathBuf::from(DEFAULT_APP)]);
        assert_eq!(got.len(), 2, "helpers are not instances");
        assert_eq!(
            got[0].data_dir,
            Some(PathBuf::from(
                "/Users/u/Library/Application Support/Claude-frontier"
            ))
        );
        assert_eq!(got[0].seat.as_deref(), Some("frontier"));
        assert_eq!(
            got[0].config_dir.as_deref(),
            Some("/Users/u/my configs/.claude-frontier")
        );
        assert_eq!(got[0].browser.as_deref(), Some("/bin/seat"));
        assert_eq!(got[1].pid, 12);
        assert_eq!((got[1].data_dir.clone(), got[1].seat.clone()), (None, None));
        let (w, rest) =
            after_words("Sun Nov  2 22:43:01 2026 /A b.app/Contents/MacOS/C --x", 5).unwrap();
        assert_eq!(
            (w.join(" ").as_str(), rest),
            ("Sun Nov 2 22:43:01 2026", "/A b.app/Contents/MacOS/C --x")
        );
    }
}
