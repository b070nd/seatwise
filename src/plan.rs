//! What each seat's subscription allows, and usage profiles sized to it: settings (main model, subagent model,
//! effort, extended thinking) merged into the seat's settings.json when you choose one.

use crate::claude::Account;
use crate::config::{Config, Seat};
use crate::usage::Usage;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// A subscription, as far as the account file tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Pro,
    Max5x,
    Max20x,
    Max,
    TeamStandard,
    TeamPremium,
    Enterprise,
    /// Anything else, by its `organizationType`.
    Other(String),
}

impl Plan {
    pub fn of(a: &Account) -> Option<Plan> {
        let kind = a.org_type.as_deref()?;
        let tier = a.rate_tier.as_deref().unwrap_or_default();
        Some(match kind {
            "claude_pro" => Plan::Pro,
            "claude_max" if tier.contains("20x") => Plan::Max20x,
            "claude_max" if tier.contains("5x") => Plan::Max5x,
            "claude_max" => Plan::Max,
            "claude_team" if a.seat_tier.as_deref() == Some("team_premium") => Plan::TeamPremium,
            "claude_team" => Plan::TeamStandard,
            "claude_enterprise" => Plan::Enterprise,
            other => Plan::Other(other.to_string()),
        })
    }

    /// Stable name, stored by `seat pin` as the seat's `plan`.
    pub fn key(&self) -> String {
        match self {
            Plan::Pro => "pro".into(),
            Plan::Max5x => "max_5x".into(),
            Plan::Max20x => "max_20x".into(),
            Plan::Max => "max".into(),
            Plan::TeamStandard => "team_standard".into(),
            Plan::TeamPremium => "team_premium".into(),
            Plan::Enterprise => "enterprise".into(),
            Plan::Other(s) => s.clone(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Plan::Pro => "Pro".into(),
            Plan::Max5x => "Max 5x".into(),
            Plan::Max20x => "Max 20x".into(),
            Plan::Max => "Max".into(),
            Plan::TeamStandard => "Team standard".into(),
            Plan::TeamPremium => "Team premium".into(),
            Plan::Enterprise => "Enterprise".into(),
            Plan::Other(s) if s == crate::claude::API_KEY => "pay per use".into(),
            Plan::Other(s) => match s.strip_prefix("chatgpt_") {
                // Codex seats: the ChatGPT plan, e.g. `chatgpt_plus` → "ChatGPT Plus".
                Some(p) => {
                    let mut c = p.chars();
                    let p = c
                        .next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default();
                    format!("ChatGPT {p}")
                }
                None => s.clone(),
            },
        }
    }

    /// The built-in profile that fits the plan's limits.
    fn fitting_profile(&self) -> &'static str {
        match self {
            Plan::Pro | Plan::TeamStandard => "light",
            Plan::Max20x => "heavy",
            _ => "balanced",
        }
    }
}

/// Built-in profiles, lightest first: (name, what it is).
pub const BUILTIN: [(&str, &str); 4] = [
    (
        "light",
        "Sonnet, Haiku subagents, low effort, no extended thinking: makes a small plan last",
    ),
    (
        "balanced",
        "Opus for planning and Sonnet for the work (opusplan), Sonnet subagents, medium effort, thinking on",
    ),
    (
        "heavy",
        "Opus, Sonnet subagents, high effort, thinking on: for plans with the most headroom",
    ),
    (
        "experimental",
        "Claude Code's newest experimental features on top of your settings: Code Mode (tool calls batched in a script)",
    ),
];

pub fn builtin(name: &str) -> Option<Value> {
    if let Some(tier) = crate::compact::profile(name) {
        // A compaction tier: only the window, on top of anything else (`--with heavy,sharp`).
        return Some(tier);
    }
    if name == "experimental" {
        // Only the features: it goes on top of a seat's own profile (`--with experimental`).
        return Some(json!({ "env": { "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE": "1" } }));
    }
    let (model, sub, effort, thinking) = match name {
        "light" => ("sonnet", "haiku", "low", false),
        "balanced" => ("opusplan", "sonnet", "medium", true),
        "heavy" => ("opus", "sonnet", "high", true),
        _ => return None,
    };
    Some(json!({
        "model": model,
        "effortLevel": effort,
        "alwaysThinkingEnabled": thinking,
        "env": { "CLAUDE_CODE_SUBAGENT_MODEL": sub },
    }))
}

/// A profile's settings: your own `[profile.<name>]` first, then the built-in one.
pub fn settings_of(cfg: &Config, name: &str) -> Option<Value> {
    match cfg.profiles.get(name) {
        Some(t) => serde_json::to_value(t).ok(),
        None => builtin(name),
    }
}

/// Every profile name with a description.
pub fn all(cfg: &Config) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = BUILTIN
        .iter()
        .filter(|(n, _)| !cfg.profiles.contains_key(*n))
        .map(|(n, d)| (n.to_string(), d.to_string()))
        .chain(
            crate::compact::TIERS
                .iter()
                .filter(|(n, _, _)| !cfg.profiles.contains_key(*n))
                .map(|(n, _, d)| (n.to_string(), format!("compaction tier: {d}"))),
        )
        .collect();
    for (name, t) in &cfg.profiles {
        out.push((
            name.clone(),
            format!("your own: {}", serde_json::to_value(t).unwrap_or_default()),
        ));
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub profile: String,
    pub why: String,
}

/// The profile that fits the plan, one step lighter when this week's limit is already mostly used.
pub fn suggest(plan: &Plan, usage: Option<&Usage>) -> Suggestion {
    let fitting = plan.fitting_profile();
    let mut why = format!("{} plan", plan.label());
    let mut profile = fitting;
    if let Some(week) = usage.and_then(Usage::weekly)
        && week >= 75.0
    {
        let i = BUILTIN.iter().position(|(n, _)| *n == fitting).unwrap_or(0);
        profile = BUILTIN[i.saturating_sub(1)].0;
        why += &format!(", {week:.0}% of this week's limit already used");
    }
    Suggestion {
        profile: profile.to_string(),
        why,
    }
}

/// Merge a profile into settings: objects (like `env`) key by key, everything else replaced. Hooks are never touched.
pub fn merge(settings: &mut Value, profile: &Value) {
    if !settings.is_object() {
        *settings = json!({});
    }
    let Some(p) = profile.as_object() else {
        return;
    };
    let s = settings.as_object_mut().expect("object");
    for (k, v) in p {
        if k == "hooks" {
            continue;
        }
        match (s.get_mut(k), v) {
            (Some(Value::Object(old)), Value::Object(new)) => {
                for (k2, v2) in new {
                    old.insert(k2.clone(), v2.clone());
                }
            }
            _ => {
                s.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Apply a profile to a seat's settings.json (the config records it separately, see `seat profile`).
pub fn apply(cfg: &Config, seat: &Seat, name: &str) -> Result<()> {
    if seat.agent != crate::config::Agent::Claude {
        bail!(
            "seat '{}' runs Codex: usage profiles are Claude Code settings. Set Codex's model and effort in {}",
            seat.name,
            seat.config_dir().join("config.toml").display()
        );
    }
    let Some(profile) = settings_of(cfg, name) else {
        bail!(
            "unknown profile '{name}' (known: {})",
            all(cfg)
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
                .join(", ")
        );
    };
    let path = seat.config_dir().join("settings.json");
    let mut settings: Value = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        Err(_) => json!({}),
    };
    merge(&mut settings, &profile);
    std::fs::create_dir_all(seat.config_dir())?;
    std::fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    crate::sync::accept(seat, &cfg.track)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(kind: &str, tier: &str, seat: Option<&str>) -> Account {
        Account {
            org_type: Some(kind.into()),
            rate_tier: Some(tier.into()),
            seat_tier: seat.map(Into::into),
            ..Account::default()
        }
    }

    #[test]
    fn plans_from_the_account_file() {
        assert_eq!(
            Plan::of(&account("claude_pro", "default_claude_ai", None)),
            Some(Plan::Pro)
        );
        assert_eq!(
            Plan::of(&account("claude_max", "default_claude_max_20x", None)),
            Some(Plan::Max20x)
        );
        assert_eq!(
            Plan::of(&account(
                "claude_team",
                "default_raven",
                Some("team_premium")
            )),
            Some(Plan::TeamPremium)
        );
        assert_eq!(
            Plan::of(&account(
                "claude_team",
                "default_raven",
                Some("team_standard")
            )),
            Some(Plan::TeamStandard)
        );
        assert_eq!(Plan::of(&Account::default()), None, "signed out");
        assert_eq!(Plan::of(&account("api", "", None)).unwrap().key(), "api");
        assert_eq!(
            Plan::of(&account("chatgpt_plus", "", None))
                .unwrap()
                .label(),
            "ChatGPT Plus"
        );
    }

    #[test]
    fn suggestion_fits_the_plan_and_steps_down_when_the_week_is_used_up() {
        assert_eq!(suggest(&Plan::Pro, None).profile, "light");
        assert_eq!(suggest(&Plan::Max20x, None).profile, "heavy");
        assert_eq!(suggest(&Plan::TeamPremium, None).profile, "balanced");
        let busy = Usage::parse(&json!({ "seven_day": { "utilization": 80.0 } }));
        let s = suggest(&Plan::Max20x, Some(&busy));
        assert_eq!(s.profile, "balanced");
        assert!(s.why.contains("80%"), "{}", s.why);
        assert_eq!(
            suggest(&Plan::Pro, Some(&busy)).profile,
            "light",
            "nothing lighter"
        );
    }

    #[test]
    fn merge_keeps_hooks_and_other_env() {
        let mut s = json!({
            "env": { "MINE": "1", "CLAUDE_CODE_SUBAGENT_MODEL": "x" },
            "hooks": { "PreToolUse": [] },
            "theme": "dark"
        });
        merge(&mut s, &builtin("light").unwrap());
        merge(&mut s, &json!({ "hooks": {} }));
        assert_eq!(s["env"]["MINE"], "1");
        assert_eq!(s["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "haiku");
        assert_eq!(s["model"], "sonnet");
        assert_eq!(s["theme"], "dark");
        assert_eq!(s["hooks"], json!({ "PreToolUse": [] }));
    }
}
