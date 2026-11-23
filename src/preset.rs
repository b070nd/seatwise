//! Use-case presets for folder groups, resolved the same way every time.
//!
//! A preset combines usage profiles (in order) and its own settings. A folder group gives a preset to its folders,
//! and the longest matching group path wins, as with seat ownership. The result reaches a session one of two ways:
//! - `session`: passed as `--settings` when seatwise starts it; nothing on disk changes.
//! - `project`: written into the project's `.claude/settings.local.json`, so a plain `claude` gets it too.
//!
//! Claude Code reads settings from several layers (user, project, local, command line); `explain` shows the result
//! and which layer set each value. A fingerprint of what seatwise adds tells sessions apart in `seat ps`.

use crate::config::{Agent, Apply, Config, Group, NONE, Seat};
use crate::folders::{key, normalize};
use crate::paths;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Built-in presets: (name, what it's for, profiles, extra settings).
pub const BUILTIN: [(&str, &str); 4] = [
    (
        "explore",
        "Reading and planning: light profile, plan mode (nothing is edited without asking)",
    ),
    (
        "build",
        "Day-to-day coding: balanced profile, file edits accepted without asking",
    ),
    ("review", "Careful review: heavy profile, plan mode"),
    ("quick", "Small fixes: light profile"),
];

fn builtin(name: &str) -> Option<(Vec<String>, Value)> {
    let (profile, extra) = match name {
        "explore" => ("light", json!({ "permissions": { "defaultMode": "plan" } })),
        "build" => (
            "balanced",
            json!({ "permissions": { "defaultMode": "acceptEdits" } }),
        ),
        "review" => ("heavy", json!({ "permissions": { "defaultMode": "plan" } })),
        "quick" => ("light", json!({})),
        _ => return None,
    };
    Some((vec![profile.to_string()], extra))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
    pub extra: Value,
    pub enabled: bool,
    pub builtin: bool,
}

/// A preset by name: your own `[preset.<name>]` first, then the built-in one.
pub fn get(cfg: &Config, name: &str) -> Option<Preset> {
    if let Some(p) = cfg.presets.get(name) {
        return Some(Preset {
            name: name.into(),
            description: p.description.clone().unwrap_or_default(),
            profiles: p.profiles.clone(),
            extra: serde_json::to_value(&p.settings).unwrap_or_else(|_| json!({})),
            enabled: p.enabled,
            builtin: false,
        });
    }
    let (profiles, extra) = builtin(name)?;
    let description = BUILTIN.iter().find(|(n, _)| *n == name)?.1.to_string();
    Some(Preset {
        name: name.into(),
        description,
        profiles,
        extra,
        enabled: true,
        builtin: true,
    })
}

/// Every preset, built-in ones first (unless replaced), then yours by name.
pub fn all(cfg: &Config) -> Vec<Preset> {
    let mut out: Vec<Preset> = BUILTIN
        .iter()
        .filter(|(n, _)| !cfg.presets.contains_key(*n))
        .filter_map(|(n, _)| get(cfg, n))
        .collect();
    out.extend(cfg.presets.keys().filter_map(|n| get(cfg, n)));
    out
}

/// A preset's settings: its profiles merged in order, then its own settings.
pub fn settings(cfg: &Config, p: &Preset) -> Value {
    let mut s = json!({});
    for prof in &p.profiles {
        if let Some(v) = crate::plan::settings_of(cfg, prof) {
            crate::plan::merge(&mut s, &v);
        }
    }
    merge_deep(&mut s, &p.extra);
    s
}

/// Like `plan::merge`, but nested objects (e.g. `permissions`) are merged at every level. Hooks are never touched.
pub fn merge_deep(into: &mut Value, add: &Value) {
    if !into.is_object() {
        *into = json!({});
    }
    let (Some(dst), Some(src)) = (into.as_object_mut(), add.as_object()) else {
        return;
    };
    for (k, v) in src {
        if k == "hooks" {
            continue;
        }
        match (dst.get_mut(k), v) {
            (Some(old @ Value::Object(_)), Value::Object(_)) => merge_deep(old, v),
            _ => {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
}

/// The group whose path is the longest match for `dir`; disabled groups are skipped.
pub fn group_for<'a>(cfg: &'a Config, dir: &Path) -> Option<&'a Group> {
    let home = paths::home();
    let dir = key(&normalize(dir, &home));
    cfg.groups
        .iter()
        .filter(|g| g.enabled)
        .flat_map(|g| {
            let home = home.clone();
            g.paths
                .iter()
                .map(move |p| (g, key(&normalize(&paths::expand(p), &home))))
        })
        .filter(|(_, p)| dir.starts_with(p))
        .max_by_key(|(_, p)| p.len())
        .map(|(g, _)| g)
}

/// The project a folder belongs to: the nearest folder up with a `.git`, else the folder itself.
pub fn project_root(dir: &Path) -> PathBuf {
    dir.ancestors()
        .find(|a| a.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}

/// What seatwise adds to a session in a folder.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub group: Option<String>,
    pub preset: Option<String>,
    pub apply: Apply,
    pub sources: Option<Vec<String>>,
    /// The preset's settings (empty when there is none).
    pub settings: Value,
    /// Why there is no preset, if there is a group.
    pub note: Option<String>,
}

pub fn resolve(cfg: &Config, seat: &Seat, dir: &Path) -> Resolved {
    let mut r = Resolved {
        settings: json!({}),
        apply: cfg.preset_apply,
        ..Resolved::default()
    };
    if seat.agent != Agent::Claude {
        return r;
    }
    let Some(g) = group_for(cfg, dir) else {
        return r;
    };
    r.group = Some(g.name.clone());
    r.apply = g.apply.unwrap_or(cfg.preset_apply);
    r.sources = g.sources.clone();
    if g.preset == NONE {
        r.note = Some(format!("group '{}' sets no preset", g.name));
        return r;
    }
    match get(cfg, &g.preset) {
        Some(p) if p.enabled => {
            r.settings = settings(cfg, &p);
            r.preset = Some(p.name);
        }
        Some(p) => r.note = Some(format!("preset '{}' is disabled", p.name)),
        None => {}
    }
    r
}

/// First 8 hex digits of the SHA-256 of what seatwise adds: same folder, same config → same fingerprint.
pub fn fingerprint(session: &Value, sources: Option<&[String]>) -> String {
    let text = format!(
        "{session}|{}",
        sources.map(|s| s.join(",")).unwrap_or_default()
    );
    Sha256::digest(text.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// `apply = "project"`: merge the preset into `<root>/.claude/settings.local.json`. Returns whether it changed.
pub fn write_project(root: &Path, preset: &Value) -> Result<bool> {
    let path = root.join(".claude").join("settings.local.json");
    let mut s = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        Err(_) => json!({}),
    };
    let before = s.clone();
    merge_deep(&mut s, preset);
    if s == before {
        return Ok(false);
    }
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(&path, serde_json::to_string_pretty(&s)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Whether every value the preset sets is already in `settings` (for drift checks).
pub fn contained(settings: &Value, preset: &Value) -> bool {
    match (settings, preset) {
        (Value::Object(s), Value::Object(p)) => p
            .iter()
            .filter(|(k, _)| *k != "hooks")
            .all(|(k, v)| s.get(k).is_some_and(|sv| contained(sv, v))),
        (a, b) => a == b,
    }
}

/// One layer of settings, lowest precedence first.
pub struct Layer {
    pub name: String,
    pub path: Option<PathBuf>,
    pub settings: Value,
}

/// The layers a seatwise-started session in `dir` gets, lowest precedence first (Claude Code: user < project <
/// local < command line). Layers turned off by the group's `sources` are left out.
pub fn layers(cfg: &Config, seat: &Seat, dir: &Path, with: &[String]) -> (Resolved, Vec<Layer>) {
    let r = resolve(cfg, seat, dir);
    let root = project_root(dir);
    let on = |s: &str| r.sources.as_ref().is_none_or(|v| v.iter().any(|x| x == s));
    let mut out = Vec::new();
    let mut file = |name: &str, path: PathBuf| {
        if on(name) {
            out.push(Layer {
                name: name.into(),
                settings: read_json(&path).unwrap_or_else(|| json!({})),
                path: Some(path),
            });
        }
    };
    file("user", seat.config_dir().join("settings.json"));
    file("project", root.join(".claude").join("settings.json"));
    file("local", root.join(".claude").join("settings.local.json"));
    if let Some(p) = &r.preset {
        match r.apply {
            Apply::Session => out.push(Layer {
                name: format!("preset {p}"),
                path: None,
                settings: r.settings.clone(),
            }),
            Apply::Project => {
                // Written into the local file on start; until then, show it where it will land.
                if let Some(local) = out.iter_mut().find(|l| l.name == "local") {
                    merge_deep(&mut local.settings, &r.settings);
                    local.name = format!("local (preset {p})");
                }
            }
        }
    }
    for w in with {
        if let Some(v) = crate::plan::settings_of(cfg, w) {
            out.push(Layer {
                name: format!("--with {w}"),
                path: None,
                settings: v,
            });
        }
    }
    (r, out)
}

/// Effective values as dotted keys with the layer that set them. Lists are combined across layers, as Claude Code
/// does for permission rules.
pub fn effective(layers: &[Layer]) -> Vec<(String, Value, String)> {
    let mut out: std::collections::BTreeMap<String, (Value, String)> = Default::default();
    fn walk(
        prefix: &str,
        v: &Value,
        layer: &str,
        out: &mut std::collections::BTreeMap<String, (Value, String)>,
    ) {
        match v {
            Value::Object(o) if !o.is_empty() => {
                for (k, v) in o {
                    if prefix.is_empty() && k == "hooks" {
                        continue;
                    }
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&key, v, layer, out);
                }
            }
            Value::Array(new) => {
                let entry = out
                    .entry(prefix.to_string())
                    .or_insert((json!([]), String::new()));
                if let Value::Array(old) = &mut entry.0 {
                    for x in new {
                        if !old.contains(x) {
                            old.push(x.clone());
                        }
                    }
                } else {
                    entry.0 = v.clone();
                }
                entry.1 = if entry.1.is_empty() {
                    layer.to_string()
                } else {
                    format!("{} + {layer}", entry.1)
                };
            }
            _ if !prefix.is_empty() => {
                out.insert(prefix.to_string(), (v.clone(), layer.to_string()));
            }
            _ => {}
        }
    }
    for l in layers {
        walk("", &l.settings, &l.name, &mut out);
    }
    out.into_iter().map(|(k, (v, l))| (k, v, l)).collect()
}

/// `seat explain`: what a session in `dir` would run with, and why.
pub fn explain(cfg: &Config, seat: &Seat, dir: &Path, with: &[String]) -> String {
    let (r, layers) = layers(cfg, seat, dir, with);
    let mut s = format!("seat     {}\n", seat.name);
    s += &format!("project  {}\n", paths::contract(&project_root(dir)));
    if seat.agent != Agent::Claude {
        return s + "presets apply to Claude Code seats only\n";
    }
    s += &format!(
        "group    {}\n",
        r.group
            .as_deref()
            .unwrap_or("- (no group covers this folder)")
    );
    let preset = match (&r.preset, &r.note) {
        (Some(p), _) => format!("{p} (apply: {})", r.apply.as_str()),
        (None, Some(n)) => format!("- ({n})"),
        (None, None) => "-".into(),
    };
    s += &format!("preset   {preset}\n");
    if let Some(src) = &r.sources {
        s += &format!("sources  {} (others ignored)\n", src.join(", "));
    }
    let mut session = if r.apply == Apply::Session {
        r.settings.clone()
    } else {
        json!({})
    };
    for w in with {
        if let Some(v) = crate::plan::settings_of(cfg, w) {
            crate::plan::merge(&mut session, &v);
        }
    }
    s += &format!("print    {}\n", fingerprint(&session, r.sources.as_deref()));
    s += "\nlayers (lowest first):\n";
    for l in &layers {
        let path = l
            .path
            .as_deref()
            .map(|p| {
                let mark = if p.exists() { "" } else { " (none)" };
                format!("  {}{mark}", paths::contract(p))
            })
            .unwrap_or_default();
        s += &format!("  {}{path}\n", l.name);
    }
    let eff = effective(&layers);
    s += "\neffective settings:\n";
    if eff.is_empty() {
        s += "  (Claude Code defaults)\n";
    }
    let width = eff.iter().map(|(k, _, _)| k.len()).max().unwrap_or(0);
    for (k, v, l) in eff {
        s += &format!("  {k:<width$} = {v}   ← {l}\n");
    }
    let mut notes = crate::catalog::check(&session, crate::catalog::Target::Session);
    if r.apply == Apply::Project {
        notes.extend(crate::catalog::check(
            &r.settings,
            crate::catalog::Target::Local,
        ));
    }
    if !notes.is_empty() {
        s += "\nfrom the settings catalog (`seat settings show <key>`):\n";
        s += &crate::catalog::report(&notes);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: &str) -> Config {
        let text = format!("[[seat]]\nname = \"w\"\nconfig_dir = \"~/.cw\"\n{extra}");
        let c: Config = toml::from_str(&text).unwrap();
        c.validate().unwrap();
        c
    }

    #[test]
    fn longest_group_wins_and_disabled_groups_fall_back() {
        let c = cfg(r#"
            [[group]]
            name = "all"
            paths = ["~/code"]
            preset = "build"
            [[group]]
            name = "legacy"
            paths = ["~/code/legacy"]
            preset = "none"
            [[group]]
            name = "off"
            paths = ["~/code/off"]
            preset = "review"
            enabled = false
        "#);
        let h = paths::home();
        let seat = &c.seats[0];
        let r = resolve(&c, seat, &h.join("code/app"));
        assert_eq!(r.preset.as_deref(), Some("build"));
        assert_eq!(r.settings["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(r.settings["model"], "opusplan");
        let r = resolve(&c, seat, &h.join("code/legacy/x"));
        assert_eq!((r.group.as_deref(), r.preset), (Some("legacy"), None));
        let r = resolve(&c, seat, &h.join("code/off"));
        assert_eq!(r.group.as_deref(), Some("all"), "disabled group is skipped");
        assert!(
            resolve(&c, seat, &h.join("codex")).group.is_none(),
            "prefix of a name is not a parent"
        );
    }

    #[test]
    fn same_input_same_fingerprint() {
        let a = json!({ "model": "opus", "env": { "A": "1" } });
        let b: Value = serde_json::from_str(r#"{"env":{"A":"1"},"model":"opus"}"#).unwrap();
        assert_eq!(fingerprint(&a, None), fingerprint(&b, None));
        assert_ne!(
            fingerprint(&a, None),
            fingerprint(&a, Some(&["user".into()]))
        );
    }

    #[test]
    fn effective_shows_the_winning_layer_and_combines_lists() {
        let layers = vec![
            Layer {
                name: "user".into(),
                path: None,
                settings: json!({ "model": "sonnet", "permissions": { "allow": ["Read"] }, "hooks": { "x": 1 } }),
            },
            Layer {
                name: "preset review".into(),
                path: None,
                settings: json!({ "model": "opus", "permissions": { "allow": ["Grep"] } }),
            },
        ];
        let eff = effective(&layers);
        assert_eq!(
            eff,
            vec![
                ("model".into(), json!("opus"), "preset review".into()),
                (
                    "permissions.allow".into(),
                    json!(["Read", "Grep"]),
                    "user + preset review".into()
                ),
            ]
        );
    }

    #[test]
    fn contained_detects_drift() {
        let p = json!({ "model": "opus", "permissions": { "defaultMode": "plan" } });
        assert!(contained(
            &json!({ "model": "opus", "x": 1, "permissions": { "defaultMode": "plan", "allow": [] } }),
            &p
        ));
        assert!(!contained(&json!({ "model": "sonnet" }), &p));
    }
}
