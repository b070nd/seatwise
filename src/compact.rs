//! Compaction tiers: when Claude Code compacts the conversation, set with its `autoCompactWindow` setting (the
//! context size, in tokens, at which it compacts). A smaller window keeps the model sharper (less old context to
//! wade through) and costs more compactions; a larger one compacts less often.
//!
//! - `sharp`     150K tokens: the smartest mode
//! - `standard`  400K tokens: the default tier
//! - `lazy`      the model's whole window (Claude Code caps any value at the model's window)
//!
//! `compact = "<tier>"` (or a token count such as `250000` or `"250k"`) in the config, globally or per seat, is
//! enforced: seatwise keeps it in the seat's settings.json (so desktop app sessions get it too) and passes it with
//! every session it starts, where it beats project and local settings. `compact = "off"` on a seat leaves its
//! settings alone. A tier is also a profile, so `seat run --with sharp`, a preset's profiles and `seat profile` take
//! it; `--with` wins for that one session.

use crate::config::{Agent, Config, Seat};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// The setting the tiers set.
pub const SETTING: &str = "autoCompactWindow";
/// Claude Code's lowest accepted value.
pub const MIN: u64 = 100_000;

/// (name, tokens, what it is), smartest first.
pub const TIERS: [(&str, u64, &str); 3] = [
    (
        "sharp",
        150_000,
        "compacts at 150K tokens: the smartest mode, with the least old context in the way",
    ),
    (
        "standard",
        400_000,
        "compacts at 400K tokens: the default tier",
    ),
    (
        "lazy",
        1_000_000,
        "compacts only when the model's whole window is full (Claude Code caps it at the window)",
    ),
];

/// A tier by name.
pub fn tier(name: &str) -> Option<u64> {
    TIERS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, t, _)| *t)
}

/// A tier as a profile's settings.
pub fn profile(name: &str) -> Option<Value> {
    tier(name).map(|t| json!({ SETTING: t }))
}

/// `"sharp"`, `"400k"`, `"250000"` → tokens; `"off"` → None. Refuses values Claude Code would not take.
pub fn parse(value: &str) -> Result<Option<u64>> {
    let v = value.trim().to_ascii_lowercase();
    if v == "off" || v == "none" {
        return Ok(None);
    }
    if let Some(t) = tier(&v) {
        return Ok(Some(t));
    }
    let (digits, scale) = match v.strip_suffix('k') {
        Some(d) => (d, 1_000),
        None => match v.strip_suffix('m') {
            Some(d) => (d, 1_000_000),
            None => (v.as_str(), 1),
        },
    };
    let n: u64 = digits.trim().replace('_', "").parse().with_context(|| {
        format!(
            "compact = \"{value}\": use sharp, standard, lazy, off, or a token count such as 250k"
        )
    })?;
    let tokens = n * scale;
    if tokens < MIN {
        bail!("compact = \"{value}\": Claude Code compacts at {MIN} tokens at the earliest");
    }
    Ok(Some(tokens))
}

/// The name of a token count when it is a tier, else e.g. "250K".
pub fn label(count: u64) -> String {
    match TIERS.iter().find(|(_, t, _)| *t == count) {
        Some((n, t, _)) => format!("{n} ({})", tokens(*t)),
        None => tokens(count),
    }
}

/// A token count, short: 150K, 1M.
pub fn tokens(tokens: u64) -> String {
    if tokens.is_multiple_of(1_000_000) {
        format!("{}M", tokens / 1_000_000)
    } else if tokens.is_multiple_of(1_000) {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// The window enforced on a seat: its own `compact`, else the global one. None: not enforced.
pub fn of(cfg: &Config, seat: &Seat) -> Option<u64> {
    if seat.agent != Agent::Claude {
        return None;
    }
    let v = seat.compact.as_ref().or(cfg.compact.as_ref())?;
    parse(&v.as_text()).ok().flatten()
}

/// Keep the seat's settings.json at the enforced window. Returns the new value when it changed something.
pub fn enforce(cfg: &Config, seat: &Seat) -> Result<Option<u64>> {
    let Some(tokens) = of(cfg, seat) else {
        return Ok(None);
    };
    let path = seat.hooks_file();
    let mut settings: Value = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        Err(_) => json!({}),
    };
    if settings[SETTING] == json!(tokens) {
        return Ok(None);
    }
    settings
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", path.display()))?
        .insert(SETTING.into(), json!(tokens));
    std::fs::create_dir_all(seat.config_dir())?;
    std::fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    crate::sync::accept(seat, &cfg.track)?; // our own change is not drift
    Ok(Some(tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_and_token_counts() {
        assert_eq!(parse("sharp").unwrap(), Some(150_000));
        assert_eq!(parse("Standard").unwrap(), Some(400_000));
        assert_eq!(parse("lazy").unwrap(), Some(1_000_000));
        assert_eq!(parse("250k").unwrap(), Some(250_000));
        assert_eq!(parse("1m").unwrap(), Some(1_000_000));
        assert_eq!(parse("300_000").unwrap(), Some(300_000));
        assert_eq!(parse("off").unwrap(), None);
        assert!(parse("50k").is_err(), "below Claude Code's minimum");
        assert!(parse("smart").is_err());
        assert_eq!(label(150_000), "sharp (150K)");
        assert_eq!(label(250_000), "250K");
    }

    #[test]
    fn tier_profiles_pass_the_catalog() {
        for (name, _, _) in TIERS {
            let p = profile(name).unwrap();
            let problems = crate::catalog::check(&p, crate::catalog::Target::Session);
            assert!(
                crate::catalog::errors(&p, crate::catalog::Target::Session).is_empty(),
                "{name}: {problems:?}"
            );
        }
    }

    #[test]
    fn the_seat_wins_over_the_global_and_off_leaves_it_alone() {
        let t = tempfile::tempdir().unwrap();
        let cfg: Config = toml::from_str(&format!(
            "compact = \"standard\"\n\
             [[seat]]\nname = \"a\"\nconfig_dir = '{0}/a'\ncompact = \"sharp\"\n\
             [[seat]]\nname = \"b\"\nconfig_dir = '{0}/b'\n\
             [[seat]]\nname = \"c\"\nconfig_dir = '{0}/c'\ncompact = \"off\"\n\
             [[seat]]\nname = \"d\"\nconfig_dir = '{0}/d'\ncompact = 250000\n",
            t.path().display()
        ))
        .unwrap();
        let seat = |n: &str| cfg.seat(n).unwrap();
        assert_eq!(of(&cfg, seat("a")), Some(150_000));
        assert_eq!(of(&cfg, seat("b")), Some(400_000));
        assert_eq!(of(&cfg, seat("c")), None);
        assert_eq!(of(&cfg, seat("d")), Some(250_000));

        std::fs::create_dir_all(t.path().join("a")).unwrap();
        std::fs::write(
            t.path().join("a/settings.json"),
            "{\"model\": \"opus\", \"autoCompactWindow\": 900000}",
        )
        .unwrap();
        assert_eq!(enforce(&cfg, seat("a")).unwrap(), Some(150_000));
        assert_eq!(enforce(&cfg, seat("a")).unwrap(), None, "already in place");
        let s: Value = serde_json::from_str(
            &std::fs::read_to_string(t.path().join("a/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(s, json!({"model": "opus", "autoCompactWindow": 150_000}));
        assert_eq!(enforce(&cfg, seat("c")).unwrap(), None);
        assert!(!t.path().join("c/settings.json").exists());
    }
}
