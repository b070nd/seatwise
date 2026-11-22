//! Settings snapshots (opt-in: config `track`, e.g. `["settings.json", "CLAUDE.md"]`). Each start compares a
//! seat's tracked files with the last known copy: a change is adopted (the old copy goes to history, so nothing
//! is lost), a deleted file is restored. Copies, not symlinks, so it works the same on every OS.

use crate::config::Seat;
use crate::paths;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

fn snapshot_dir(seat: &Seat) -> PathBuf {
    paths::data_dir().join("snapshots").join(&seat.name)
}

fn history_dir(seat: &Seat) -> PathBuf {
    paths::data_dir().join("history").join(&seat.name)
}

fn write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, data).with_context(|| format!("writing {}", path.display()))
}

/// Keep `data` as the newest history entry for `file`.
fn keep(seat: &Seat, file: &str, data: &[u8]) -> Result<PathBuf> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let mut n = 0;
    loop {
        let path = history_dir(seat).join(format!("{secs:012}-{n:03}-{file}"));
        if !path.exists() {
            write(&path, data)?;
            return Ok(path);
        }
        n += 1;
    }
}

/// Compare with the last snapshot; returns notes for the user.
pub fn reconcile(seat: &Seat, track: &[String]) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    for file in track {
        let live = seat.config_dir().join(file);
        let snap = snapshot_dir(seat).join(file);
        match (fs::read(&live).ok(), fs::read(&snap).ok()) {
            (Some(l), None) => write(&snap, &l)?,
            (Some(l), Some(s)) if l != s => {
                let kept = keep(seat, file, &s)?;
                write(&snap, &l)?;
                notes.push(format!(
                    "{file} of seat '{}' changed since the last start; the previous version is kept at {} (undo: seat sync restore {})",
                    seat.name,
                    kept.display(),
                    seat.name
                ));
            }
            (None, Some(s)) => {
                write(&live, &s)?;
                notes.push(format!(
                    "{file} of seat '{}' was missing; restored the last known version",
                    seat.name
                ));
            }
            _ => {}
        }
    }
    Ok(notes)
}

/// Take the current files as the snapshot without a note (after seatwise itself changed them).
pub fn accept(seat: &Seat, track: &[String]) -> Result<()> {
    for file in track {
        if let Ok(data) = fs::read(seat.config_dir().join(file)) {
            write(&snapshot_dir(seat).join(file), &data)?;
        }
    }
    Ok(())
}

/// Swap each tracked file back to its newest history entry (the current one goes to history, so running it again undoes it).
pub fn restore(seat: &Seat, track: &[String]) -> Result<Vec<String>> {
    if track.is_empty() {
        bail!("no files are tracked; set track = [\"settings.json\", ...] in the config");
    }
    let mut notes = Vec::new();
    let entries: Vec<PathBuf> = match fs::read_dir(history_dir(seat)) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => bail!("no history for seat '{}' yet", seat.name),
    };
    for file in track {
        let suffix = format!("-{file}");
        let Some(newest) = entries
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(&suffix))
            })
            .max()
        else {
            continue;
        };
        let live = seat.config_dir().join(file);
        let old = fs::read(newest)?;
        if let Ok(current) = fs::read(&live) {
            keep(seat, file, &current)?;
        }
        write(&live, &old)?;
        write(&snapshot_dir(seat).join(file), &old)?;
        fs::remove_file(newest)?;
        notes.push(format!("restored {file} of seat '{}'", seat.name));
    }
    if notes.is_empty() {
        bail!("no history for seat '{}' yet", seat.name);
    }
    Ok(notes)
}
