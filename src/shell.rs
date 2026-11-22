//! `seat shell-init <shell>`: a function `<prefix><seat>` for each seat and alias (prefix `claude-` by default),
//! and, with `shell.override_claude`, plain `claude` picking the seat from the current folder.

use crate::config::Config;
use clap::ValueEnum;
use std::path::Path;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
    Powershell,
}

pub fn init(shell: Shell, cfg: &Config, exe: &Path) -> String {
    let exe = exe.display().to_string();
    let mut names: Vec<(String, String)> = Vec::new();
    if cfg.shell.override_claude {
        names.push(("claude".into(), "auto".into()));
    }
    for seat in cfg.seats.iter().filter(|s| cfg.is_enabled(s)) {
        for name in std::iter::once(&seat.name).chain(&seat.aliases) {
            let prefix = match seat.agent {
                crate::config::Agent::Claude => &cfg.shell.prefix,
                crate::config::Agent::Codex => &cfg.shell.codex_prefix,
            };
            names.push((format!("{prefix}{name}"), seat.name.clone()));
        }
    }
    let mut out = String::from("# seatwise: Claude Code seats\n");
    for (func, seat) in names {
        out += &match shell {
            // `function NAME` rather than `NAME()`: the shell would expand NAME as an alias while parsing
            // `NAME()` (zsh: "defining function based on alias", then a parse error that aborts the whole eval),
            // and the `unalias` on the same line runs too late to prevent it.
            Shell::Zsh | Shell::Bash => {
                format!(
                    "unalias {func} 2>/dev/null; function {func} {{ {} run {seat} \"$@\"; }}\n",
                    sh_quote(&exe)
                )
            }
            Shell::Fish => format!(
                "function {func}; {} run {seat} $argv; end\n",
                sh_quote(&exe)
            ),
            Shell::Powershell => {
                format!(
                    "function {func} {{ & '{}' run {seat} @args }}\n",
                    exe.replace('\'', "''")
                )
            }
        };
    }
    out
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Seat;

    #[test]
    fn functions_for_every_seat_and_alias() {
        let cfg = Config {
            seats: vec![Seat {
                name: "work".into(),
                aliases: vec!["w".into()],
                config_dir: "~/.claude-work".into(),
                ..Seat::default()
            }],
            ..Config::default()
        };
        let zsh = init(Shell::Zsh, &cfg, Path::new("/opt/it's/seat"));
        assert!(
            zsh.contains(r#"function claude-work { '/opt/it'\''s/seat' run work "$@"; }"#),
            "{zsh}"
        );
        assert!(zsh.contains("function claude-w {"));
        assert!(!zsh.contains("claude()"), "plain claude is opt-in: {zsh}");
        let mut opted = cfg.clone();
        opted.shell.override_claude = true;
        opted.shell.prefix = "cc-".into();
        let fish = init(Shell::Fish, &opted, Path::new("/bin/seat"));
        assert!(
            fish.contains("function claude; '/bin/seat' run auto $argv; end"),
            "{fish}"
        );
        assert!(fish.contains("function cc-w;"), "{fish}");
        let ps = init(Shell::Powershell, &cfg, Path::new(r"C:\x\seat.exe"));
        assert!(
            ps.contains(r"function claude-work { & 'C:\x\seat.exe' run work @args"),
            "{ps}"
        );
    }
}
