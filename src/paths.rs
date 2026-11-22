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

fn xdg(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
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
}
