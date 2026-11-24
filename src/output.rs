//! Everything seatwise prints goes through here.
//!
//! This is the one place that decides whether a line may leave the machine as it is. When output can reach an
//! agent or isn't a terminal (see [`crate::privacy::active`]), each line is redacted before it is written: account
//! identifiers become placeholders such as `<org:work>`. Commands print with [`outln!`], [`out!`], [`errln!`] and
//! [`err!`] and never think about it, so a new command cannot forget. A few commands hand the terminal to something
//! else (an agent session, the TUI, shell code that is evaluated); they call [`passthrough`] first.
//!
//! Writes never panic: a closed pipe (`seat list | head`) just ends the output quietly.

use crate::config::Config;
use crate::paths;
use std::borrow::Cow;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

type Known = Arc<Vec<(String, String)>>;

static PASSTHROUGH: AtomicBool = AtomicBool::new(false);
static ACTIVE: OnceLock<bool> = OnceLock::new();
/// The known identifiers, with the config file's modification time they were read at: a command that changes the
/// config (`seat pin`, `seat add`) refreshes them before it prints what it did.
static KNOWN: Mutex<Option<(Option<SystemTime>, Known)>> = Mutex::new(None);

/// This command prints what it must show as it is (an agent session, the TUI, shell code to evaluate).
pub fn passthrough() {
    PASSTHROUGH.store(true, Ordering::SeqCst);
}

/// Whether lines are being redacted.
pub fn redacting() -> bool {
    !PASSTHROUGH.load(Ordering::SeqCst) && *ACTIVE.get_or_init(crate::privacy::active)
}

fn known() -> Known {
    let stamp = std::fs::metadata(paths::config_file())
        .and_then(|m| m.modified())
        .ok();
    if let Some((at, k)) = KNOWN.lock().ok().as_deref().and_then(Option::as_ref)
        && *at == stamp
    {
        return k.clone();
    }
    // Outside the lock: loading the config must never wait on, or print through, this module.
    let fresh: Known = Arc::new(crate::privacy::known(&Config::load().unwrap_or_default()));
    if let Ok(mut g) = KNOWN.lock() {
        *g = Some((stamp, fresh.clone()));
    }
    fresh
}

fn render(text: &str) -> Cow<'_, str> {
    if redacting() {
        Cow::Owned(crate::privacy::redact(&known(), text))
    } else {
        Cow::Borrowed(text)
    }
}

/// Write `text` to standard output.
pub fn out(text: &str) {
    let text = render(text);
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(text.as_bytes());
    let _ = o.flush();
}

/// Write `text` to standard error.
pub fn err(text: &str) {
    let text = render(text);
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(text.as_bytes());
    let _ = e.flush();
}

/// A writer for code that takes `impl Write` (the preset wizard): complete lines go through the redactor.
#[derive(Default)]
pub struct Stdout {
    pending: Vec<u8>,
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buf);
        if let Some(i) = self.pending.iter().rposition(|b| *b == b'\n') {
            let rest = self.pending.split_off(i + 1);
            out(&String::from_utf8_lossy(&self.pending));
            self.pending = rest;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.pending.is_empty() {
            out(&String::from_utf8_lossy(&std::mem::take(&mut self.pending)));
        }
        Ok(())
    }
}

/// `println!`, through the redactor.
#[macro_export]
macro_rules! outln {
    () => { $crate::output::out("\n") };
    ($($arg:tt)*) => { $crate::output::out(&format!("{}\n", format_args!($($arg)*))) };
}

/// `print!`, through the redactor.
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => { $crate::output::out(&format!($($arg)*)) };
}

/// `eprintln!`, through the redactor.
#[macro_export]
macro_rules! errln {
    () => { $crate::output::err("\n") };
    ($($arg:tt)*) => { $crate::output::err(&format!("{}\n", format_args!($($arg)*))) };
}

/// `eprint!`, through the redactor.
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => { $crate::output::err(&format!($($arg)*)) };
}
