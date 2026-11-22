//! `seat usage`: how much of each seat's rate limits (5-hour and weekly windows) is used.
//!
//! This reads the seat's own Claude Code login (`.credentials.json`, or on macOS the keychain item Claude Code
//! keeps for that config folder) and asks the same endpoint Claude Code's `/usage` uses. That endpoint is not
//! documented and may change. The token is only sent to that endpoint, never shown or stored, and never
//! refreshed: refreshing would rotate it and could sign Claude Code out.

use crate::config::Seat;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// As the endpoint names it, e.g. `five_hour`, `seven_day`, `seven_day_opus`.
    pub name: String,
    /// Percent used.
    pub utilization: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub windows: Vec<Window>,
}

impl Usage {
    /// Every top-level object with a numeric `utilization`.
    pub fn parse(v: &Value) -> Usage {
        let mut windows: Vec<Window> = v
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(name, w)| {
                Some(Window {
                    name: name.clone(),
                    utilization: w.get("utilization")?.as_f64()?,
                    resets_at: w.get("resets_at").and_then(Value::as_str).map(Into::into),
                })
            })
            .collect();
        windows.sort_by_key(|w| (!w.name.starts_with("five"), w.name.clone()));
        Usage { windows }
    }

    /// The fullest weekly window.
    pub fn weekly(&self) -> Option<f64> {
        self.windows
            .iter()
            .filter(|w| w.name.starts_with("seven_day"))
            .map(|w| w.utilization)
            .reduce(f64::max)
    }
}

/// `5h`, `7d`, `7d opus`, ...
pub fn window_label(name: &str) -> String {
    let rest = name
        .strip_prefix("five_hour")
        .map(|r| ("5h", r))
        .or_else(|| name.strip_prefix("seven_day").map(|r| ("7d", r)));
    match rest {
        Some((head, tail)) => format!("{head}{}", tail.replace('_', " ")),
        None => name.replace('_', " "),
    }
}

/// The keychain item Claude Code uses for a config folder: plain for the default location, else suffixed with
/// the first 8 hex digits of the SHA-256 of the folder's path.
pub fn keychain_service(seat: &Seat) -> String {
    if seat.is_default_location() {
        return "Claude Code-credentials".into();
    }
    let hash = Sha256::digest(seat.config_dir().display().to_string().as_bytes());
    let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("Claude Code-credentials-{hex}")
}

struct Token {
    access: String,
    expires_ms: Option<i64>,
}

fn token_from(json: &str) -> Option<Token> {
    let v: Value = serde_json::from_str(json).ok()?;
    let o = &v["claudeAiOauth"];
    Some(Token {
        access: o["accessToken"].as_str()?.to_string(),
        expires_ms: o["expiresAt"].as_i64(),
    })
}

fn read_token(seat: &Seat) -> Result<Token> {
    if let Ok(text) = std::fs::read_to_string(seat.config_dir().join(".credentials.json")) {
        return token_from(&text).context("the seat's .credentials.json has no Claude login");
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", &keychain_service(seat), "-w"])
            .stderr(std::process::Stdio::null())
            .output()
            .context("running security")?;
        if out.status.success() {
            return token_from(&String::from_utf8_lossy(&out.stdout))
                .context("the keychain item has no Claude login");
        }
    }
    bail!(
        "seat '{}' has no Claude login (`seat login {}`)",
        seat.name,
        seat.name
    )
}

/// The seat's current usage.
pub fn fetch(seat: &Seat) -> Result<Usage> {
    if seat.agent != crate::config::Agent::Claude {
        bail!(
            "seat '{}' runs Codex: its usage is shown by `/status` inside Codex",
            seat.name
        );
    }
    let token = read_token(seat)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    if token.expires_ms.is_some_and(|e| e < now_ms) {
        bail!(
            "seat '{}': its login token has expired; start the seat once (Claude Code refreshes it), then try again",
            seat.name
        );
    }
    let url = std::env::var("SEATWISE_USAGE_URL").unwrap_or_else(|_| ENDPOINT.into());
    if !allowed_url(&url) {
        bail!(
            "refusing to send the seat's login anywhere but {ENDPOINT} (SEATWISE_USAGE_URL may only point at this computer)"
        );
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .get(&url)
        .header("Authorization", &format!("Bearer {}", token.access))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(
            "User-Agent",
            &format!("seatwise/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("asking {url}"))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    match status {
        200 => {
            let v: Value = serde_json::from_str(&body).context("unreadable usage answer")?;
            Ok(Usage::parse(&v))
        }
        401 | 403 => bail!(
            "seat '{}': the usage endpoint refused the login (HTTP {status}); start the seat once to refresh it",
            seat.name
        ),
        _ => bail!(
            "seat '{}': usage endpoint answered HTTP {status}",
            seat.name
        ),
    }
}

/// The login token goes to Anthropic's own endpoint, or (for tests) to this computer; nowhere else.
pub fn allowed_url(url: &str) -> bool {
    if url == ENDPOINT {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// `2026-11-22T14:05:00Z` → `2026-11-22 14:05 UTC`; anything else as is.
fn short_time(t: &str) -> String {
    if t.len() >= 16 && t.as_bytes()[10] == b'T' {
        let utc = if t.ends_with('Z') || t.contains("+00:00") {
            " UTC"
        } else {
            ""
        };
        format!("{} {}{utc}", &t[..10], &t[11..16])
    } else {
        t.to_string()
    }
}

/// One line per window: `5h  23%  ████░░░░░░░░  resets 2026-11-22 14:05 UTC`.
pub fn render(usage: &Usage) -> Vec<String> {
    usage
        .windows
        .iter()
        .map(|w| {
            let filled = ((w.utilization / 100.0 * 12.0).round() as usize).min(12);
            let bar = format!("{}{}", "█".repeat(filled), "░".repeat(12 - filled));
            let reset = w
                .resets_at
                .as_deref()
                .map(|r| format!("  resets {}", short_time(r)))
                .unwrap_or_default();
            format!(
                "{:<8} {:>4.0}%  {bar}{reset}",
                window_label(&w.name),
                w.utilization
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_windows_and_finds_the_fullest_week() {
        let u = Usage::parse(&json!({
            "seven_day": { "utilization": 40.0, "resets_at": "2026-11-25T08:00:00Z" },
            "five_hour": { "utilization": 12.5, "resets_at": null },
            "seven_day_opus": { "utilization": 61.0 },
            "extra_usage": { "is_enabled": false },
            "seven_day_oauth_apps": null
        }));
        let names: Vec<_> = u.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["five_hour", "seven_day", "seven_day_opus"]);
        assert_eq!(u.weekly(), Some(61.0));
        assert_eq!(window_label("seven_day_opus"), "7d opus");
        let lines = render(&u);
        assert!(
            lines[1].contains("40%") && lines[1].contains("resets 2026-11-25 08:00 UTC"),
            "{lines:?}"
        );
    }

    #[test]
    fn the_token_only_goes_to_anthropic_or_this_computer() {
        assert!(allowed_url(ENDPOINT));
        assert!(allowed_url("http://127.0.0.1:4321/usage"));
        assert!(allowed_url("http://localhost/x"));
        for bad in [
            "https://api.anthropic.com.evil.io/api/oauth/usage",
            "http://127.0.0.1.evil.io/",
            "http://evil.io/?127.0.0.1",
            "https://localhost/",
            "http://user@evil.io:80/",
        ] {
            assert!(!allowed_url(bad), "{bad}");
        }
    }

    #[test]
    fn keychain_names_follow_claude_code() {
        let seat = |dir: &str| Seat {
            name: "w".into(),
            config_dir: dir.into(),
            ..Seat::default()
        };
        assert_eq!(
            keychain_service(&seat("~/.claude")),
            "Claude Code-credentials"
        );
        // sha256("/home/u/.claude-work") starts with 9fa37e90.
        assert_eq!(
            keychain_service(&seat("/home/u/.claude-work")),
            "Claude Code-credentials-9fa37e90"
        );
    }
}
