//! Reading and writing Claude Code's own files: the signed-in account and the hooks in settings.json.

use crate::config::{Agent, Config, Enforcement, Seat};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub account_uuid: Option<String>,
    pub email: Option<String>,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
    /// `organizationType`, e.g. `claude_pro`, `claude_max`, `claude_team`, `claude_enterprise`.
    pub org_type: Option<String>,
    /// `organizationRateLimitTier`, e.g. `default_claude_max_20x`.
    pub rate_tier: Option<String>,
    /// `seatTier` on team plans, e.g. `team_standard`, `team_premium`.
    pub seat_tier: Option<String>,
    /// Usage beyond the plan is billed (`hasExtraUsageEnabled`).
    pub extra_usage: bool,
    /// `claudeCodeTrialEndsAt`, as given.
    pub trial_ends: Option<String>,
}

/// `org_type` of a login by API key (billed per use, no plan).
pub const API_KEY: &str = "api_key";

/// The account a seat is signed into, from Claude Code's state file. `None` if signed out.
pub fn read_account(seat: &Seat) -> Option<Account> {
    match seat.agent {
        Agent::Claude => read_account_from(&seat.state_file()),
        Agent::Codex => read_codex_account(&seat.state_file()),
    }
}

/// A Codex login (`$CODEX_HOME/auth.json`): the ChatGPT account from the `id_token`'s claims. The file's layout is
/// not documented, so anything unexpected reads as "no account". An API-key login (`OPENAI_API_KEY` set, or
/// `auth_mode` naming an API key) is an account without an email or org, with the plan [`API_KEY`]. The key itself
/// is only checked for being there, never kept.
pub fn read_codex_account(auth_file: &Path) -> Option<Account> {
    let auth: Value = serde_json::from_str(&std::fs::read_to_string(auth_file).ok()?).ok()?;
    let Some(token) = auth["tokens"]["id_token"].as_str() else {
        let has_key = auth["OPENAI_API_KEY"]
            .as_str()
            .is_some_and(|k| !k.trim().is_empty());
        let mode = auth["auth_mode"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let key_mode = mode.replace(['_', '-'], "") == "apikey";
        return (has_key || key_mode).then(|| Account {
            org_type: Some(API_KEY.into()),
            ..Account::default()
        });
    };
    let payload = token.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    let openai = &claims["https://api.openai.com/auth"];
    let s = |v: &Value| v.as_str().map(str::to_owned);
    Some(Account {
        account_uuid: s(&claims["sub"]),
        email: s(&claims["email"]),
        org_uuid: s(&openai["chatgpt_account_id"]).or_else(|| s(&auth["tokens"]["account_id"])),
        org_type: s(&openai["chatgpt_plan_type"]).map(|p| format!("chatgpt_{p}")),
        ..Account::default()
    })
}

/// Decode unpadded base64url (JWT segments).
fn base64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The account in a Claude Code state file (`.claude.json`).
pub fn read_account_from(state_file: &Path) -> Option<Account> {
    let state: Value = serde_json::from_str(&std::fs::read_to_string(state_file).ok()?).ok()?;
    let account = state.get("oauthAccount")?;
    let field = |k: &str| account[k].as_str().map(str::to_owned);
    Some(Account {
        account_uuid: field("accountUuid"),
        email: field("emailAddress"),
        org_uuid: field("organizationUuid"),
        org_name: field("organizationName"),
        org_type: field("organizationType"),
        rate_tier: field("organizationRateLimitTier"),
        seat_tier: field("seatTier"),
        extra_usage: account["hasExtraUsageEnabled"].as_bool().unwrap_or(false),
        trial_ends: field("claudeCodeTrialEndsAt"),
    })
}

/// Tools whose input is checked against the folder assignments (config `tool_matcher` overrides).
pub const TOOL_MATCHER: &str =
    "Read|Edit|Write|MultiEdit|NotebookEdit|Glob|Grep|LS|Bash|PowerShell";

/// The version of what seatwise's hooks expect from the host and promise to it: the hook events and their input
/// fields, exit 2 to block, the JSON for a warning. It is stamped into each hook command (`--contract N`), so
/// `seat doctor` and the launcher can tell hooks written for another contract. Raise it when that changes.
pub const HOOK_CONTRACT: u32 = 1;

fn hook_command(exe: &Path, kind: &str, seat: &str) -> String {
    format!(
        "\"{}\" hook {kind} {seat} --contract {HOOK_CONTRACT}",
        exe.display()
    )
}

/// A seatwise hook command taken apart: binary, kind (`prompt`/`tool`), seat, contract (none before 0.8.0).
fn parse_hook(command: &str) -> Option<(PathBuf, &str, &str, Option<u32>)> {
    let rest = command.strip_prefix('"')?;
    let (exe, rest) = rest.split_once("\" hook ")?;
    let mut words = rest.split_whitespace();
    let kind = words.next()?;
    let seat = words.next()?;
    let contract = match (words.next(), words.next()) {
        (Some("--contract"), Some(n)) => Some(n.parse().ok()?),
        (None, _) => None,
        _ => return None,
    };
    Some((PathBuf::from(exe), kind, seat, contract))
}

/// The same file, also when reached through another path (a symlink such as `~/.local/bin/seat`).
fn same_binary(a: &Path, b: &Path) -> bool {
    a == b || matches!((dunce::canonicalize(a), dunce::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// The hook contract a seat's installed hooks were written for (`None`: no hooks, or written before contracts).
pub fn hook_contract(seat: &Seat) -> Option<u32> {
    our_hooks(seat)?
        .iter()
        .filter_map(|c| parse_hook(c).and_then(|h| h.3))
        .min()
}

fn is_ours(command: &str) -> bool {
    command.contains("\" hook prompt ") || command.contains("\" hook tool ")
}

/// Whether a seat's settings.json has seatwise's hooks, and whether they point at this binary and seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookState {
    Missing,
    Current,
    /// Installed by another `seat` binary path or for another seat name: run `seat install-hooks`.
    Stale,
}

/// The commands of seatwise's hooks in a seat's settings.json (`None` when it can't be read).
fn our_hooks(seat: &Seat) -> Option<Vec<String>> {
    let settings = std::fs::read_to_string(seat.hooks_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())?;
    let mut ours = Vec::new();
    for event in ["UserPromptSubmit", "PreToolUse"] {
        for group in settings["hooks"][event].as_array().into_iter().flatten() {
            for h in group["hooks"].as_array().into_iter().flatten() {
                if let Some(c) = h["command"].as_str().filter(|c| is_ours(c)) {
                    ours.push(c.to_string());
                }
            }
        }
    }
    Some(ours)
}

pub fn hook_state(seat: &Seat, exe: &Path) -> HookState {
    let Some(ours) = our_hooks(seat) else {
        return HookState::Missing;
    };
    let current = |kind: &str| {
        ours.iter()
            .filter_map(|c| parse_hook(c))
            .any(|(e, k, s, c)| {
                k == kind && s == seat.name && c == Some(HOOK_CONTRACT) && same_binary(&e, exe)
            })
    };
    match ours.len() {
        0 => HookState::Missing,
        2 if current("prompt") && current("tool") => HookState::Current,
        _ => HookState::Stale,
    }
}

/// Remove seatwise's hook entries (and groups and event lists left empty by that), keeping everything else.
pub fn remove_hooks(settings: &mut Value) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for event in ["UserPromptSubmit", "PreToolUse"] {
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        groups.retain_mut(
            |group| match group.get_mut("hooks").and_then(Value::as_array_mut) {
                Some(entries) => {
                    let before = entries.len();
                    entries.retain(|h| !h["command"].as_str().is_some_and(is_ours));
                    before == 0 || !entries.is_empty()
                }
                None => true,
            },
        );
        if groups.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        settings.as_object_mut().unwrap().remove("hooks");
    }
}

/// Put seatwise's two hooks into `settings`, replacing earlier seatwise entries and keeping everything else.
pub fn merge_hooks(settings: &mut Value, exe: &Path, seat: &str, matcher: Option<&str>) {
    if !settings.is_object() {
        *settings = json!({});
    }
    if !settings["hooks"].is_object() {
        settings["hooks"] = json!({});
    }
    remove_hooks(settings);
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    for (event, kind, matcher) in [
        ("UserPromptSubmit", "prompt", None),
        ("PreToolUse", "tool", matcher),
    ] {
        let list = hooks
            .as_object_mut()
            .unwrap()
            .entry(event)
            .or_insert_with(|| json!([]));
        if !list.is_array() {
            *list = json!([]);
        }
        let mut group =
            json!({ "hooks": [{ "type": "command", "command": hook_command(exe, kind, seat) }] });
        if let Some(m) = matcher {
            group["matcher"] = json!(m);
        }
        list.as_array_mut().unwrap().push(group);
    }
}

/// Install, refresh or (when the seat's enforcement is off) remove the guard hooks in a seat's settings.json.
/// Returns whether hooks are now installed.
pub fn install_hooks(cfg: &Config, seat: &Seat, exe: &Path) -> Result<bool> {
    let wanted = cfg.enforcement_of(seat) != Enforcement::Off;
    let path = seat.hooks_file();
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !wanted => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let before = settings.clone();
    if wanted {
        // Codex names its tools differently: check every call (the tool hook scans any input it gets).
        let matcher = match seat.agent {
            Agent::Claude => Some(cfg.tool_matcher.as_deref().unwrap_or(TOOL_MATCHER)),
            Agent::Codex => None,
        };
        merge_hooks(&mut settings, exe, &seat.name, matcher);
    } else {
        remove_hooks(&mut settings);
    }
    if settings != before {
        std::fs::create_dir_all(seat.config_dir())?;
        std::fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
        crate::sync::accept(seat, &cfg.track)?; // our own change is not drift
    }
    Ok(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_account_from_the_id_token() {
        let t = tempfile::tempdir().unwrap();
        // {"sub":"u1","email":"me@example.org","https://api.openai.com/auth":{"chatgpt_plan_type":"plus","chatgpt_account_id":"acc-1"}}
        let payload = "eyJzdWIiOiJ1MSIsImVtYWlsIjoibWVAZXhhbXBsZS5vcmciLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9wbGFuX3R5cGUiOiJwbHVzIiwiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjLTEifX0";
        let file = t.path().join("auth.json");
        std::fs::write(
            &file,
            json!({ "tokens": { "id_token": format!("h.{payload}.s"), "account_id": "acc-1" } })
                .to_string(),
        )
        .unwrap();
        let a = read_codex_account(&file).unwrap();
        assert_eq!(a.email.as_deref(), Some("me@example.org"));
        assert_eq!(a.org_uuid.as_deref(), Some("acc-1"));
        assert_eq!(a.org_type.as_deref(), Some("chatgpt_plus"));
        std::fs::write(&file, r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-x"}"#).unwrap();
        let a = read_codex_account(&file).unwrap();
        assert_eq!(a.org_type.as_deref(), Some(API_KEY), "API-key login");
        assert_eq!((a.email, a.org_uuid), (None, None));
        std::fs::write(&file, r#"{"OPENAI_API_KEY":null}"#).unwrap();
        assert_eq!(
            read_codex_account(&file),
            None,
            "no key, no tokens: signed out"
        );
    }

    #[test]
    fn merge_is_idempotent_and_keeps_user_settings() {
        let mut s = json!({
            "model": "x",
            "hooks": { "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "my-linter" }] },
                { "matcher": "Read", "hooks": [{ "type": "command", "command": "\"/old/seat\" hook tool w" }] }
            ]}
        });
        merge_hooks(&mut s, Path::new("/bin/seat"), "w", Some(TOOL_MATCHER));
        merge_hooks(&mut s, Path::new("/bin/seat"), "w", Some(TOOL_MATCHER));
        assert_eq!(s["model"], "x");
        let pre = s["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(
            pre.len(),
            2,
            "user group kept, old seatwise group replaced: {pre:?}"
        );
        assert_eq!(pre[0]["hooks"][0]["command"], "my-linter");
        assert_eq!(
            pre[1]["hooks"][0]["command"],
            format!("\"/bin/seat\" hook tool w --contract {HOOK_CONTRACT}")
        );
        assert_eq!(pre[1]["matcher"], TOOL_MATCHER);
        assert_eq!(s["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);

        remove_hooks(&mut s);
        assert_eq!(
            s,
            json!({
                "model": "x",
                "hooks": { "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "my-linter" }] }
                ]}
            }),
            "only the user's own hook is left"
        );
        let mut only_ours = json!({});
        merge_hooks(&mut only_ours, Path::new("/bin/seat"), "w", Some("Read"));
        remove_hooks(&mut only_ours);
        assert_eq!(only_ours, json!({}), "no empty leftovers");
    }

    #[test]
    fn hook_commands_carry_the_contract() {
        let cmd = hook_command(Path::new("/my dir/seat"), "prompt", "w");
        assert_eq!(
            parse_hook(&cmd),
            Some((
                PathBuf::from("/my dir/seat"),
                "prompt",
                "w",
                Some(HOOK_CONTRACT)
            ))
        );
        assert_eq!(
            parse_hook("\"/old/seat\" hook tool w"),
            Some((PathBuf::from("/old/seat"), "tool", "w", None)),
            "hooks from before contracts"
        );
        assert_eq!(parse_hook("my-linter"), None);
    }
}
