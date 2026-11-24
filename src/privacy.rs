//! Account identifiers stay on this computer.
//!
//! seatwise's output can contain org UUIDs, org names, account emails and account ids. When an agent runs seatwise
//! (a skill, a hook, a tool call), that output goes into the model's context and leaves the machine. So when
//! output may reach an agent, every identifier is replaced by a placeholder naming the seat it belongs to,
//! such as `<org:work>`, and any other UUID or email address by `<uuid>` / `<email>`.
//!
//! Redaction is on when an agent is detected (Claude Code, Codex, a session seatwise started), or when output
//! isn't a terminal. `SEATWISE_REVEAL=1` turns it off, but only outside an agent: an agent can't reveal for itself.
//! Credentials are never printed at all, redacted or not.

use crate::config::Config;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::PathBuf;

/// Set by Claude Code (`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`), by Codex in its sandboxed commands, and by
/// seatwise for the sessions it starts (`SEATWISE_SEAT`).
const AGENT_VARS: [&str; 5] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "SEATWISE_SEAT",
];

pub fn agent() -> bool {
    AGENT_VARS.iter().any(|v| std::env::var_os(v).is_some())
        || std::env::var_os("SEATWISE_PRIVATE").is_some()
}

/// Whether output must be redacted.
pub fn active() -> bool {
    if agent() {
        return true;
    }
    if std::env::var("SEATWISE_REVEAL").is_ok_and(|v| v == "1") {
        return false;
    }
    !(std::io::stdout().is_terminal() && std::io::stderr().is_terminal())
}

/// Known identifiers, longest first, with their placeholders.
pub fn known(cfg: &Config) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |value: Option<&str>, kind: &str, seat: &str| {
        if let Some(v) = value.filter(|v| v.len() >= 3) {
            out.push((v.to_string(), format!("<{kind}:{seat}>")));
        }
    };
    for s in &cfg.seats {
        add(s.org_uuid.as_deref(), "org", &s.name);
        add(s.org_name.as_deref(), "org-name", &s.name);
        add(s.email.as_deref(), "email", &s.name);
        if let Some(a) = crate::claude::read_account(s) {
            add(a.org_uuid.as_deref(), "org", &s.name);
            add(a.org_name.as_deref(), "org-name", &s.name);
            add(a.email.as_deref(), "email", &s.name);
            add(a.account_uuid.as_deref(), "account", &s.name);
        }
    }
    // Accounts in config folders no seat names yet (e.g. before `seat init`).
    for (name, a) in crate::init::accounts() {
        add(a.org_uuid.as_deref(), "org", &name);
        add(a.org_name.as_deref(), "org-name", &name);
        add(a.email.as_deref(), "email", &name);
        add(a.account_uuid.as_deref(), "account", &name);
    }
    // Paths seatwise itself prints (config file, data folder, seat folders, project folders) stay as they are:
    // a placeholder inside a path makes it unusable. A value equal to its placeholder means "keep".
    let mut keep = |p: PathBuf| {
        let s = p.display().to_string();
        if s.len() >= 3 {
            out.push((s.clone(), s));
        }
    };
    keep(crate::paths::config_file());
    keep(crate::paths::data_dir());
    for s in &cfg.seats {
        keep(s.config_dir());
    }
    for f in &cfg.folders {
        keep(crate::paths::expand(&f.path));
    }
    // A value shared by two seats (same email in two orgs) keeps the first seat's placeholder; the point is that
    // the value is gone.
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// `8-4-4-4-12` hex digits at `i`.
fn uuid_at(b: &[u8], i: usize) -> bool {
    let groups = [8, 4, 4, 4, 12];
    let mut j = i;
    for (n, len) in groups.iter().enumerate() {
        if j + len > b.len() || !b[j..j + len].iter().all(|&c| is_hex(c)) {
            return false;
        }
        j += len;
        if n < 4 {
            if b.get(j) != Some(&b'-') {
                return false;
            }
            j += 1;
        }
    }
    let boundary = |k: Option<&u8>| k.is_none_or(|c| !c.is_ascii_alphanumeric());
    boundary(i.checked_sub(1).and_then(|k| b.get(k))) && boundary(b.get(j))
}

fn email_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._%+-".contains(c)
}

/// Replace UUIDs and email addresses that aren't known.
fn generic(s: &str) -> String {
    // UUIDs.
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if uuid_at(b, i) {
            out += "<uuid>";
            i += 36;
        } else {
            let ch = s[i..].chars().next().expect("in bounds");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    // Emails: local@domain.tld.
    let chars: Vec<char> = out.chars().collect();
    let mut res = String::with_capacity(out.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let local_len = res.chars().rev().take_while(|c| email_char(*c)).count();
            let mut j = i + 1;
            while j < chars.len() && email_char(chars[j]) {
                j += 1;
            }
            let domain: String = chars[i + 1..j].iter().collect();
            let domain = domain.trim_end_matches('.');
            if local_len > 0 && domain.contains('.') && !domain.starts_with('.') {
                for _ in 0..local_len {
                    res.pop();
                }
                res += "<email>";
                i += 1 + domain.chars().count();
                continue;
            }
        }
        res.push(chars[i]);
        i += 1;
    }
    res
}

pub fn redact(known: &[(String, String)], line: &str) -> String {
    // Entries whose value is their own placeholder are paths to keep: redact around them.
    let keeps = || {
        known
            .iter()
            .filter(|(v, p)| v == p)
            .map(|(v, _)| v.as_str())
    };
    let first = keeps()
        .filter_map(|k| line.find(k).map(|i| (i, k)))
        .min_by_key(|(i, k)| (*i, std::cmp::Reverse(k.len())));
    if let Some((i, k)) = first {
        return format!(
            "{}{k}{}",
            redact(known, &line[..i]),
            redact(known, &line[i + k.len()..])
        );
    }
    let mut s = line.to_string();
    for (value, placeholder) in known {
        if s.contains(value.as_str()) {
            s = s.replace(value.as_str(), placeholder);
        }
    }
    generic(&s)
}

/// Run this same command again with its output redacted line by line. Returns its exit code.
pub fn filtered(cfg: &Config) -> std::io::Result<i32> {
    let known = std::sync::Arc::new(known(cfg));
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        .env("SEATWISE_FILTERED", "1")
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let err = child.stderr.take().expect("piped");
    let k = known.clone();
    let t = std::thread::spawn(move || copy(BufReader::new(err), &mut std::io::stderr(), &k));
    copy(
        BufReader::new(child.stdout.take().expect("piped")),
        &mut std::io::stdout(),
        &known,
    );
    let _ = t.join();
    Ok(child.wait()?.code().unwrap_or(1))
}

fn copy(mut from: impl BufRead, to: &mut impl Write, known: &[(String, String)]) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match from.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let _ = to.write_all(redact(known, &line).as_bytes());
                let _ = to.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_name_their_seat_and_the_rest_is_generic() {
        let known = vec![
            ("Example Corp".to_string(), "<org-name:work>".to_string()),
            ("me@example.com".to_string(), "<email:work>".to_string()),
        ];
        let line = "work  me@example.com (Example Corp)  other: you@x.org id 12345678-90ab-4cde-8f01-234567890abc";
        assert_eq!(
            redact(&known, line),
            "work  <email:work> (<org-name:work>)  other: <email> id <uuid>"
        );
        assert_eq!(
            redact(&[], "a path ~/x/y@2 and 1234"),
            "a path ~/x/y@2 and 1234"
        );
        assert_eq!(redact(&[], "ü <a@b.co>."), "ü <<email>>.");
        let kept = vec![(
            "/h/12345678-90ab-4cde-8f01-234567890abc/c.toml".to_string(),
            "/h/12345678-90ab-4cde-8f01-234567890abc/c.toml".to_string(),
        )];
        assert_eq!(
            redact(
                &kept,
                "config /h/12345678-90ab-4cde-8f01-234567890abc/c.toml and 12345678-90ab-4cde-8f01-234567890abc"
            ),
            "config /h/12345678-90ab-4cde-8f01-234567890abc/c.toml and <uuid>",
            "own paths stay usable"
        );
        assert_eq!(
            redact(&[], "x12345678-90ab-4cde-8f01-234567890abc"),
            "x12345678-90ab-4cde-8f01-234567890abc",
            "not a whole UUID"
        );
    }
}
