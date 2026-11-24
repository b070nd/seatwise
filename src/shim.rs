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

/// Whether a fresh login shell finds the shim first: what a new terminal (or a tool starting one) would get.
/// `None` when that can't be told (Windows, no `$SHELL`, the shell didn't answer in time).
pub fn first_in_login_shell() -> Option<bool> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var_os("SHELL")?;
    let mut child = std::process::Command::new(shell)
        .args(["-lic", "printf '%s' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                return None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    // The last line: an interactive shell may print a banner first.
    Some(first_in(std::ffi::OsStr::new(out.lines().last()?.trim())))
}

/// Whether the shim folder comes before every other folder with a `claude` on PATH.
pub fn first_on_path() -> bool {
    first_in(&std::env::var_os("PATH").unwrap_or_default())
}

fn first_in(path: &std::ffi::OsStr) -> bool {
    let shim = crate::folders::key_string(&dir());
    for d in std::env::split_paths(path) {
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
