//! Where things live. Every location can be overridden with an env var (used by the tests).

use std::path::PathBuf;

/// Home folder (`SEATWISE_HOME` overrides).
pub fn home() -> PathBuf {
    std::env::var_os("SEATWISE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .expect("cannot determine the home folder")
}

/// Expand a leading home reference: `~`, `$HOME`, `${HOME}`, and the Windows forms
/// `%USERPROFILE%` / `$env:USERPROFILE` (case-insensitive). Anything else is returned as is.
pub fn expand(s: &str) -> PathBuf {
    const PREFIXES: [(&str, bool); 5] = [
        ("~", false),
        ("$HOME", false),
        ("${HOME}", false),
        ("%USERPROFILE%", true),
        ("$env:USERPROFILE", true),
    ];
    for (prefix, ignore_case) in PREFIXES {
        let Some(head) = s.get(..prefix.len()) else {
            continue;
        };
        let matches = if ignore_case {
            head.eq_ignore_ascii_case(prefix)
        } else {
            head == prefix
        };
        if !matches {
            continue;
        }
        let rest = &s[prefix.len()..];
        if rest.is_empty() {
            return home();
        }
        if let Some(rest) = rest.strip_prefix(['/', '\\']) {
            return home().join(rest);
        }
    }
    PathBuf::from(s)
}

/// The short form of a path for the config file: `~/...` under the home folder (with `/`), else as is.
pub fn contract(p: &std::path::Path) -> String {
    let home = home();
    match p.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => {
            let parts: Vec<_> = rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            format!("~/{}", parts.join("/"))
        }
        Err(_) => p.display().to_string(),
    }
}

/// `config.toml` (`SEATWISE_CONFIG` overrides): `~/.config/seatwise/` on Unix, `%APPDATA%\seatwise\` on Windows.
pub fn config_file() -> PathBuf {
    if let Some(p) = std::env::var_os("SEATWISE_CONFIG") {
        return p.into();
    }
    let base = if cfg!(windows) {
        dirs::config_dir().unwrap_or_else(home)
    } else {
        xdg("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
    };
    base.join("seatwise").join("config.toml")
}

/// Snapshots and history (`SEATWISE_DATA` overrides): `~/.local/share/seatwise` or `%LOCALAPPDATA%\seatwise`.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SEATWISE_DATA") {
        return p.into();
    }
    let base = if cfg!(windows) {
        dirs::data_local_dir().unwrap_or_else(home)
    } else {
        xdg("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share"))
    };
    base.join("seatwise")
}

/// An XDG base folder, unless it points into a temporary or per-session folder: some agent sessions (the desktop
/// app's, for one) set `XDG_CONFIG_HOME` to their scratch workspace, and seatwise would then read and write a
/// throwaway config there instead of the user's.
fn xdg(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && !session_scoped(p))
}

/// Inside a temp folder or a per-session scratch workspace.
pub fn session_scoped(p: &std::path::Path) -> bool {
    let temp = [
        std::env::temp_dir(),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/private/var/folders"),
        PathBuf::from("/var/folders"),
    ];
    temp.iter().any(|t| p.starts_with(t))
        || p.components().any(|c| {
            let c = c.as_os_str().to_string_lossy().to_ascii_lowercase();
            c.starts_with("scratch-workspace") || c == "scratchpad"
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_forms() {
        let h = home();
        assert_eq!(expand("~"), h);
        assert_eq!(expand("~/a/b"), h.join("a/b"));
        assert_eq!(expand("$HOME/a"), h.join("a"));
        assert_eq!(expand("${HOME}/a"), h.join("a"));
        assert_eq!(expand("%userprofile%\\a"), h.join("a"));
        assert_eq!(expand("$env:USERPROFILE\\a"), h.join("a"));
        assert_eq!(expand("~other/a"), PathBuf::from("~other/a"));
        assert_eq!(expand("/abs"), PathBuf::from("/abs"));
        assert_eq!(contract(&h.join("a").join("b")), "~/a/b");
        assert_eq!(expand(&contract(&h.join("a"))), h.join("a"));
    }

    #[test]
    fn session_folders_are_not_config_homes() {
        assert!(session_scoped(std::path::Path::new(
            "/Users/u/Library/Application Support/Claude/local-agent-mode-sessions/scratch-workspaces-x/home/.config"
        )));
        assert!(session_scoped(&std::env::temp_dir().join("x")));
        assert!(!session_scoped(std::path::Path::new("/Users/u/.config")));
    }
}
