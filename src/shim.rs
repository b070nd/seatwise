//! A `claude` command on PATH that runs `seat run auto`, so tools that start `claude` themselves (IDE terminals,
//! JetBrains, scripts) get the seat that owns the folder too. It lives in its own folder, which the launcher
//! skips when it looks for the real `claude`.

use crate::paths;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The folder that holds the shim (`<data dir>/bin`).
pub fn dir() -> PathBuf {
    paths::data_dir().join("bin")
}

pub fn file() -> PathBuf {
    dir().join(if cfg!(windows) {
        "claude.cmd"
    } else {
        "claude"
    })
}

pub fn script(exe: &Path) -> String {
    if cfg!(windows) {
        format!(
            "@echo off\r\nrem seatwise shim: picks the seat that owns the current folder\r\n\"{}\" run auto %*\r\n",
            exe.display()
        )
    } else {
        format!(
            "#!/bin/sh\n# seatwise shim: picks the seat that owns the current folder\nexec '{}' run auto \"$@\"\n",
            exe.display().to_string().replace('\'', r"'\''")
        )
    }
}

pub fn install(exe: &Path) -> Result<PathBuf> {
    let f = file();
    std::fs::create_dir_all(dir())?;
    std::fs::write(&f, script(exe)).with_context(|| format!("writing {}", f.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(f)
}

pub fn uninstall() -> Result<bool> {
    match std::fs::remove_file(file()) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn installed() -> bool {
    file().is_file()
}

/// Whether the shim folder comes before every other folder with a `claude` on PATH.
pub fn first_on_path() -> bool {
    let shim = crate::folders::key_string(&dir());
    for d in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if crate::folders::key_string(&d) == shim {
            return true;
        }
        if ["claude", "claude.exe", "claude.cmd"]
            .iter()
            .any(|n| d.join(n).is_file())
        {
            return false;
        }
    }
    false
}
