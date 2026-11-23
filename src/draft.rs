//! The Preset Draft: build a preset's settings one choice at a time, so it can only end up valid.
//!
//! A draft starts empty, from a preset, or from a profile. [`Draft::options`] offers only settings Claude Code honors
//! where the preset will be applied, marking those that would clash with what is already chosen;
//! [`Draft::set`] takes a choice only if it adds no error or warning from the Settings Catalog (experimental notes
//! are fine); [`Draft::preview`] is the merged result; [`Draft::save`] writes `[preset.<name>]`, keeping comments.
//! The `seat preset` wizard and the TUI editor both drive a draft.

use crate::catalog::{self, Entry, Kind, Level, Problem, Target};
use crate::config::{Apply, Config};
use crate::edit::{ConfigFile, json_to_toml};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
    /// The preset's own settings (on top of its profiles), as Claude Code's settings JSON.
    pub settings: Value,
    /// Where the preset will be applied: session (`--settings`) or local (`apply = "project"`).
    pub target: Target,
}

/// A setting the draft can offer.
#[derive(Debug, Clone)]
pub struct Choice {
    pub entry: &'static Entry,
    /// Its value in the draft now, if set.
    pub current: Option<Value>,
    /// The values a person picks from (enums, flags, booleans); empty when any value of its type goes.
    pub values: Vec<String>,
    /// Why it can't be chosen now, if it can't (a documented clash with something already set).
    pub blocked: Option<String>,
}

/// Where a preset is applied: `local` when any enabled group applies it to the project's files.
pub fn target_of(cfg: &Config, name: &str) -> Target {
    let project = cfg.groups.iter().any(|g| {
        g.enabled && g.preset == name && g.apply.unwrap_or(cfg.preset_apply) == Apply::Project
    });
    if project {
        Target::Local
    } else {
        Target::Session
    }
}

fn path_of(e: &Entry) -> Vec<String> {
    match e.kind() {
        Kind::Env => vec!["env".into(), e.name.clone()],
        Kind::Setting => e.name.split('.').map(str::to_string).collect(),
    }
}

fn get_path<'a>(v: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(k))
}

fn set_path(v: &mut Value, path: &[String], value: Option<Value>) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut cur = v;
    for k in parents {
        if !cur.is_object() {
            *cur = json!({});
        }
        cur = cur
            .as_object_mut()
            .expect("object")
            .entry(k.clone())
            .or_insert_with(|| json!({}));
    }
    if !cur.is_object() {
        *cur = json!({});
    }
    let map = cur.as_object_mut().expect("object");
    match value {
        Some(x) => {
            map.insert(last.clone(), x);
        }
        None => {
            map.remove(last);
        }
    }
}

/// Drop objects left empty by removals (`permissions = {}`).
fn prune(v: &mut Value) {
    if let Some(map) = v.as_object_mut() {
        for child in map.values_mut() {
            prune(child);
        }
        map.retain(|_, c| !c.as_object().is_some_and(|o| o.is_empty()));
    }
}

/// The values to pick from for an entry.
pub fn values_of(e: &Entry) -> Vec<String> {
    match e.ty.as_str() {
        "enum" => e.values.clone(),
        "flag" => vec!["1".into(), "0".into()],
        "bool" if e.kind() == Kind::Env => vec!["1".into(), "0".into()],
        "bool" => vec!["true".into(), "false".into()],
        _ => Vec::new(),
    }
}

/// A value typed by a person, as the JSON the entry takes. Environment variables are always strings.
pub fn parse_value(e: &Entry, raw: &str) -> Result<Value> {
    let raw = raw.trim();
    if e.kind() == Kind::Env {
        return Ok(Value::String(raw.to_string()));
    }
    Ok(match e.ty.as_str() {
        "bool" => match raw {
            "true" | "on" | "yes" | "1" => Value::Bool(true),
            "false" | "off" | "no" | "0" => Value::Bool(false),
            _ => bail!("{} takes true or false", e.name),
        },
        "number" => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(|n| {
                if raw.contains('.') {
                    Value::Number(n)
                } else {
                    json!(raw.parse::<i64>().unwrap_or_default())
                }
            })
            .with_context(|| format!("{} takes a number", e.name))?,
        "list" if !raw.starts_with('[') => Value::Array(
            raw.split(',')
                .map(|s| Value::String(s.trim().to_string()))
                .filter(|s| s.as_str() != Some(""))
                .collect(),
        ),
        "list" | "object" | "any" => serde_json::from_str(raw)
            .or_else(|_| serde_json::from_str(&format!("\"{raw}\"")))
            .with_context(|| format!("{} takes JSON", e.name))?,
        _ => Value::String(raw.to_string()),
    })
}

impl Draft {
    pub fn new(name: &str) -> Self {
        Draft {
            name: name.into(),
            description: String::new(),
            profiles: Vec::new(),
            settings: json!({}),
            target: Target::Session,
        }
    }

    /// A draft of an existing preset (yours or a built-in), to change or to copy under a new name.
    pub fn from_preset(cfg: &Config, preset: &str, name: &str) -> Result<Self> {
        let p = crate::preset::get(cfg, preset)
            .with_context(|| format!("unknown preset '{preset}' (`seat preset list`)"))?;
        Ok(Draft {
            name: name.into(),
            description: p.description,
            profiles: p.profiles,
            settings: if p.extra.is_object() {
                p.extra
            } else {
                json!({})
            },
            target: target_of(cfg, name),
        })
    }

    /// A draft whose settings are a profile's (yours or a built-in), with no profiles of its own.
    pub fn from_profile(cfg: &Config, profile: &str, name: &str) -> Result<Self> {
        let settings = crate::plan::settings_of(cfg, profile)
            .with_context(|| format!("unknown profile '{profile}' (`seat profile`)"))?;
        Ok(Draft {
            name: name.into(),
            description: format!("from profile {profile}"),
            profiles: Vec::new(),
            settings,
            target: target_of(cfg, name),
        })
    }

    /// The settings a session gets from this preset: its profiles merged in order, then its own settings.
    pub fn preview(&self, cfg: &Config) -> Value {
        let p = crate::preset::Preset {
            name: self.name.clone(),
            description: self.description.clone(),
            profiles: self.profiles.clone(),
            extra: self.settings.clone(),
            enabled: true,
            builtin: false,
        };
        crate::preset::settings(cfg, &p)
    }

    /// What the Settings Catalog says about the preview where the preset is applied.
    pub fn problems(&self, cfg: &Config) -> Vec<Problem> {
        catalog::check(&self.preview(cfg), self.target)
    }

    fn blocking(problems: &[Problem]) -> Vec<&Problem> {
        problems.iter().filter(|p| p.level != Level::Info).collect()
    }

    /// Settings matching `query` that this draft can take: honored where it is applied, not deprecated, one value
    /// each (no whole objects that have keys of their own). Already-set ones come first.
    pub fn options(&self, cfg: &Config, query: &str, experimental: bool) -> Vec<Choice> {
        let mut out: Vec<Choice> = catalog::search(
            query,
            catalog::Filter {
                experimental,
                ..Default::default()
            },
        )
        .into_iter()
        .filter(|e| e.ty != "removed" && !(e.ty == "object" && has_children(e)))
        .filter(|e| e.name != "hooks" && e.name != "env")
        .filter(|e| {
            let exported = self.target == Target::Session
                && e.kind() == Kind::Env
                && e.scopes.iter().any(|s| s == "shell");
            exported || e.scopes.iter().any(|s| s == target_scope(self.target))
        })
        .map(|e| {
            let current = get_path(&self.settings, &path_of(e)).cloned();
            let values = values_of(e);
            let blocked = if current.is_none() {
                self.clash(cfg, e, &values)
            } else {
                None
            };
            Choice {
                entry: e,
                current,
                values,
                blocked,
            }
        })
        .collect();
        out.sort_by_key(|c| c.current.is_none());
        out
    }

    /// A reason no value of `e` fits next to what is set, when every candidate value would add a problem.
    fn clash(&self, cfg: &Config, e: &'static Entry, values: &[String]) -> Option<String> {
        if values.is_empty() {
            return None;
        }
        let before = self.problems(cfg);
        let mut reason = None;
        for v in values {
            let mut next = self.clone();
            let Ok(value) = parse_value(e, v) else {
                continue;
            };
            set_path(&mut next.settings, &path_of(e), Some(value));
            let added: Vec<Problem> = Self::blocking(&next.problems(cfg))
                .into_iter()
                .filter(|p| !before.contains(p))
                .cloned()
                .collect();
            // A value that adds nothing means no clash.
            reason = Some(added.first()?.message.clone());
        }
        reason
    }

    /// Set a key (a settings key or an environment variable) from a typed value. Refused, with the reason, when the
    /// key is unknown or the value would add an error or warning. Returns the notes it brings (experimental).
    pub fn set(&mut self, cfg: &Config, key: &str, raw: &str) -> Result<Vec<Problem>> {
        let Some(e) = catalog::lookup(key) else {
            let hint = catalog::near(key)
                .first()
                .map(|e| format!("; did you mean {}?", e.name))
                .unwrap_or_default();
            bail!("{key} is not in the settings catalog{hint} (`seat settings search`)");
        };
        if e.ty == "removed" || e.deprecated() {
            bail!("{key} is deprecated or removed: {}", e.meaning);
        }
        let value = parse_value(e, raw)?;
        let before = self.problems(cfg);
        let mut next = self.clone();
        set_path(&mut next.settings, &path_of(e), Some(value));
        let after = next.problems(cfg);
        let added: Vec<&Problem> = Self::blocking(&after)
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect();
        if let Some(p) = added.first() {
            bail!("not set: {}", p.message);
        }
        *self = next;
        Ok(after
            .into_iter()
            .filter(|p| p.level == Level::Info && p.key == e.name)
            .collect())
    }

    /// Remove a key. Removing never makes a draft invalid in a way setting could have avoided, so it always works.
    pub fn unset(&mut self, key: &str) -> bool {
        let path = match catalog::lookup(key) {
            Some(e) => path_of(e),
            None if key
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') =>
            {
                vec!["env".into(), key.into()]
            }
            None => key.split('.').map(str::to_string).collect(),
        };
        let had = get_path(&self.settings, &path).is_some();
        set_path(&mut self.settings, &path, None);
        prune(&mut self.settings);
        had
    }

    /// Use these profiles (in order) under the preset's own settings. Unknown names are refused.
    pub fn set_profiles(&mut self, cfg: &Config, profiles: &[String]) -> Result<()> {
        for p in profiles {
            if crate::plan::settings_of(cfg, p).is_none() {
                bail!("unknown profile '{p}' (`seat profile` lists them)");
            }
        }
        let mut next = self.clone();
        next.profiles = profiles.to_vec();
        let before = self.problems(cfg);
        if let Some(p) = Self::blocking(&next.problems(cfg))
            .into_iter()
            .find(|p| !before.contains(p))
        {
            bail!("not changed: {}", p.message);
        }
        *self = next;
        Ok(())
    }

    /// Write `[preset.<name>]` (description, profiles, settings), keeping the rest of the file as it is.
    pub fn save(&self, cfg: &Config, file: &mut ConfigFile) -> Result<()> {
        if let Some(p) = Self::blocking(&self.problems(cfg)).first() {
            bail!("preset '{}' not saved: {}", self.name, p.message);
        }
        if crate::config::NONE == self.name {
            bail!("'{}' is reserved", self.name);
        }
        file.set_preset(
            &self.name,
            &self.description,
            &self.profiles,
            &json_to_toml(&self.settings),
        )
    }

    /// A short summary: description, profiles, and each own setting.
    pub fn summary(&self) -> String {
        let mut s = format!("preset {}", self.name);
        if !self.description.is_empty() {
            s += &format!(": {}", self.description);
        }
        s += &format!(
            "\n  applied in  {}\n  profiles    {}\n",
            self.target.label(),
            if self.profiles.is_empty() {
                "-".into()
            } else {
                self.profiles.join(" + ")
            }
        );
        let mut rows = Vec::new();
        flat(&self.settings, "", &mut rows);
        if rows.is_empty() {
            s += "  settings    (none of its own)\n";
        }
        for (k, v) in rows {
            let mark = if catalog::lookup(&k).is_some_and(|e| e.experimental()) {
                "  (experimental)"
            } else {
                ""
            };
            s += &format!("  {k} = {v}{mark}\n");
        }
        s
    }
}

fn flat(v: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    for (k, x) in v.as_object().into_iter().flatten() {
        let key = if prefix.is_empty() || prefix == "env" {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        let next = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match x {
            Value::Object(_) if catalog::lookup(&next).is_none_or(|e| e.ty == "object") => {
                flat(x, &next, out)
            }
            _ => out.push((key, x.to_string())),
        }
    }
}

fn has_children(e: &Entry) -> bool {
    let prefix = format!("{}.", e.name);
    !catalog::search(
        "",
        catalog::Filter {
            deprecated: true,
            ..Default::default()
        },
    )
    .iter()
    .all(|x| !x.name.starts_with(&prefix))
}

fn target_scope(t: Target) -> &'static str {
    match t {
        Target::Session => "session",
        Target::Local => "local",
        Target::User => "user",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        toml::from_str("").unwrap()
    }

    #[test]
    fn only_valid_choices_are_taken() {
        let c = cfg();
        let mut d = Draft::new("fast");
        assert!(d.set(&c, "effortLevel", "hihg").is_err());
        assert!(
            d.set(&c, "efortLevel", "high")
                .unwrap_err()
                .to_string()
                .contains("did you mean effortLevel")
        );
        d.set(&c, "effortLevel", "high").unwrap();
        let notes = d
            .set(&c, "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE", "1")
            .unwrap();
        assert!(
            notes.iter().any(|n| n.message.contains("experimental")),
            "{notes:?}"
        );
        // A documented clash is refused.
        let err = d
            .set(&c, "CLAUDE_CODE_EFFORT_LEVEL", "low")
            .unwrap_err()
            .to_string();
        assert!(err.contains("overrides effortLevel"), "{err}");
        d.set(&c, "permissions.defaultMode", "plan").unwrap();
        assert_eq!(d.settings["permissions"]["defaultMode"], "plan");
        assert_eq!(d.settings["env"]["CLAUDE_CODE_EXPERIMENTAL_CODE_MODE"], "1");
        assert!(d.unset("permissions.defaultMode"));
        assert!(
            d.settings.get("permissions").is_none(),
            "empty tables go: {}",
            d.settings
        );
    }

    #[test]
    fn project_presets_are_offered_only_what_project_files_honor() {
        let c = cfg();
        let mut d = Draft::new("p");
        d.target = Target::Local;
        assert!(
            d.set(&c, "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE", "1")
                .is_err()
        );
        assert!(d.set(&c, "permissions.defaultMode", "auto").is_err());
        d.set(&c, "permissions.defaultMode", "acceptEdits").unwrap();
        let offered = d.options(&c, "code mode", true);
        assert!(
            offered
                .iter()
                .all(|o| o.entry.name != "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE")
        );
        let session = Draft::new("s").options(&c, "code mode", true);
        assert!(
            session
                .iter()
                .any(|o| o.entry.name == "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE")
        );
    }

    #[test]
    fn clashing_choices_are_marked() {
        let c = cfg();
        let mut d = Draft::new("x");
        d.set(&c, "model", "sonnet").unwrap();
        let o = d.options(&c, "ANTHROPIC_MODEL", false);
        let m = o
            .iter()
            .find(|o| o.entry.name == "ANTHROPIC_MODEL")
            .unwrap();
        assert!(m.values.is_empty(), "free text: not checked ahead");
        let mut d = Draft::new("y");
        d.set(&c, "alwaysThinkingEnabled", "false").unwrap();
        assert!(d.set(&c, "MAX_THINKING_TOKENS", "4096").is_err());
    }
}
