//! What seatwise decided, kept on disk: so "why was that blocked?" has an answer after the message has scrolled away.
//!
//! Two kinds of log, both in `<data dir>/logs`:
//! - A **run log** per invocation (`seat-<date>-<time>.<micros>.log`), created at the first line and rotated: only the
//!   newest [`MAX_RUN_LOGS`] are kept. `seat run` writes what it checked and decided, and any command that fails
//!   writes its error; the error output then ends with the log's path.
//! - `hooks.log`, one file for every hook call: blocks and warnings only (allowing is the normal case and would drown
//!   the rest), capped at [`HOOKS_LOG_BYTES`] and rotated once to `hooks.log.1`. `seat log` shows its end.
//!
//! Lines are passed through the redactor first and hold folder paths and seat names, never identifiers, credentials
//! or the arguments of a session. Writing is best effort: a log that can't be written never fails a command.

use crate::paths;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Run logs kept, counting the one being written.
pub const MAX_RUN_LOGS: usize = 50;
/// Size at which `hooks.log` is moved to `hooks.log.1`.
pub const HOOKS_LOG_BYTES: u64 = 256 * 1024;

static RUN: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn dir() -> PathBuf {
    paths::data_dir().join("logs")
}

fn hooks_file() -> PathBuf {
    dir().join("hooks.log")
}

/// (year, month, day, hour, minute, second) in UTC for seconds since the Unix epoch.
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

fn now() -> (u64, u32) {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (t.as_secs(), t.subsec_micros())
}

/// `2026-11-24 15:04:05` (UTC).
pub fn stamp() -> String {
    let (y, mo, d, h, mi, s) = civil(now().0);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// Remove the oldest `seat-*.log` files in `dir` until `keep` are left.
fn rotate(dir: &Path, keep: usize) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("seat-") && n.ends_with(".log"))
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(keep);
    for old in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

fn new_run_file(dir: &Path) -> PathBuf {
    let (secs, micros) = now();
    let (y, mo, d, h, mi, s) = civil(secs);
    let _ = std::fs::create_dir_all(dir);
    rotate(dir, MAX_RUN_LOGS - 1);
    dir.join(format!(
        "seat-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.{micros:06}.log"
    ))
}

fn append(path: &Path, text: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(text.as_bytes());
    }
}

/// One line in this run's log, created on first use.
pub fn line(message: &str) {
    let Ok(mut run) = RUN.lock() else { return };
    let path = run.get_or_insert_with(|| new_run_file(&dir())).clone();
    drop(run);
    append(
        &path,
        &format!("{} {}\n", stamp(), crate::output::scrub(message)),
    );
}

/// This run's log, if anything was written to it.
pub fn path() -> Option<PathBuf> {
    RUN.lock().ok()?.clone()
}

/// A block or warning from a hook, appended to `hooks.log`.
pub fn hook_event(message: &str) {
    let file = hooks_file();
    let _ = std::fs::create_dir_all(dir());
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > HOOKS_LOG_BYTES) {
        let _ = std::fs::rename(&file, file.with_extension("log.1"));
    }
    append(
        &file,
        &format!("{} {}\n", stamp(), crate::output::scrub(message)),
    );
}

/// The last `n` lines of `hooks.log`.
pub fn tail(n: usize) -> Vec<String> {
    let text = std::fs::read_to_string(hooks_file()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_time_matches_known_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951_782_400 + 86_399), (2000, 2, 29, 23, 59, 59));
        assert_eq!(civil(1_795_530_245), (2026, 11, 24, 14, 24, 5));
    }

    #[test]
    fn rotation_keeps_the_newest_and_ignores_other_files() {
        let t = tempfile::tempdir().unwrap();
        for n in [
            "seat-20260101-000000.000001.log",
            "seat-20260102-000000.000001.log",
            "seat-20260103-000000.000001.log",
            "hooks.log",
            "notes.txt",
        ] {
            std::fs::write(t.path().join(n), "x").unwrap();
        }
        rotate(t.path(), 2);
        let mut left: Vec<String> = std::fs::read_dir(t.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "hooks.log",
                "notes.txt",
                "seat-20260102-000000.000001.log",
                "seat-20260103-000000.000001.log"
            ]
        );
    }
}
