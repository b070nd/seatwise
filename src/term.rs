//! `seat term`: open a new terminal window running a seat's CLI in a project folder, optionally with a
//! session-only usage profile (e.g. experimental settings). This is the hybrid way of working: chat in the
//! desktop app, and when the work needs the CLI, open it for the same project on the same seat.

use crate::config::Config;
use crate::paths;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn cmd_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// The `seat run ...` invocation, as words.
pub fn run_words(
    exe: &Path,
    seat: &str,
    profiles: &[String],
    home: bool,
    extra: &[String],
) -> Vec<String> {
    let mut w = vec![exe.display().to_string(), "run".into()];
    for p in profiles {
        w.push("--with".into());
        w.push(p.clone());
    }
    if home {
        w.push("--home".into());
    }
    w.push(seat.into());
    w.extend(extra.iter().cloned());
    w
}

/// A shell command line that enters `dir` and runs `words`.
pub fn shell_line(dir: &Path, words: &[String]) -> String {
    if cfg!(windows) {
        let cmd: Vec<String> = words.iter().map(|w| cmd_quote(w)).collect();
        format!(
            "cd /d {} && {}",
            cmd_quote(&dir.display().to_string()),
            cmd.join(" ")
        )
    } else {
        let cmd: Vec<String> = words.iter().map(|w| sh_quote(w)).collect();
        format!(
            "cd {} && exec {}",
            sh_quote(&dir.display().to_string()),
            cmd.join(" ")
        )
    }
}

/// Open a terminal window running `line` in `dir`.
pub fn open(cfg: &Config, dir: &Path, line: &str) -> Result<()> {
    if !cfg.terminal.command.is_empty() {
        let argv: Vec<String> = cfg
            .terminal
            .command
            .iter()
            .map(|a| {
                a.replace("{dir}", &dir.display().to_string())
                    .replace("{cmd}", line)
            })
            .collect();
        return spawn(Command::new(paths::expand(&argv[0])).args(&argv[1..]));
    }
    if cfg!(target_os = "macos") {
        // A .command file opens in the terminal app as a new window, with no scripting permission needed.
        let dir_s = paths::data_dir().join("term");
        std::fs::create_dir_all(&dir_s)?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let file = dir_s.join(format!("seat-{secs}.command"));
        std::fs::write(
            &file,
            format!(
                "#!/bin/sh\nrm -f {}\n{line}\n",
                sh_quote(&file.display().to_string())
            ),
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))?;
        }
        let app = cfg.terminal.app.as_deref().unwrap_or("Terminal");
        return spawn(Command::new("open").args(["-a", app]).arg(&file));
    }
    if cfg!(windows) {
        if spawn(
            Command::new("wt")
                .arg("-d")
                .arg(dir)
                .args(["cmd", "/k", line]),
        )
        .is_ok()
        {
            return Ok(());
        }
        return spawn(Command::new("cmd").args(["/c", "start", "", "cmd", "/k", line]));
    }
    let tries: [(&str, &[&str]); 5] = [
        ("x-terminal-emulator", &["-e", "sh", "-c"]),
        ("gnome-terminal", &["--", "sh", "-c"]),
        ("konsole", &["-e", "sh", "-c"]),
        ("kitty", &["sh", "-c"]),
        ("xterm", &["-e", "sh", "-c"]),
    ];
    for (prog, args) in tries {
        if spawn(Command::new(prog).args(args).arg(line)).is_ok() {
            return Ok(());
        }
    }
    bail!(
        "no terminal found; set terminal.command in the config, e.g. [\"foot\", \"sh\", \"-c\", \"{{cmd}}\"]"
    )
}

fn spawn(cmd: &mut Command) -> Result<()> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .with_context(|| format!("starting {:?}", cmd.get_program()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_everything() {
        let words = run_words(
            Path::new("/opt/se at/seat"),
            "work",
            &["experimental".into()],
            false,
            &["--continue".into()],
        );
        assert_eq!(
            words,
            [
                "/opt/se at/seat",
                "run",
                "--with",
                "experimental",
                "work",
                "--continue"
            ]
        );
        if !cfg!(windows) {
            assert_eq!(
                shell_line(Path::new("/p/it's"), &words),
                r#"cd '/p/it'\''s' && exec '/opt/se at/seat' 'run' '--with' 'experimental' 'work' '--continue'"#
            );
        }
    }
}
