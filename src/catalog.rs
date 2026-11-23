//! The Settings Catalog: every Claude Code setting and environment variable seatwise knows, with its type, allowed
//! values, where Claude Code honors it, whether it is experimental, and the combinations Claude Code documents.
//!
//! Callers ask three things: [`lookup`] a key, [`search`] the catalog, [`check`] a settings value for the place it
//! will be applied. The data is `catalog/claude-code.toml`, taken from Claude Code's settings and env-var docs.
//! Keys the catalog doesn't know are a warning, never an error, so a new Claude Code release never breaks a config.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::LazyLock;

const DATA: &str = include_str!("../catalog/claude-code.toml");

/// When the catalog was last checked against Claude Code's docs.
pub const CHECKED: &str = "2026-11-23 (Claude Code docs up to v2.1.330)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A `settings.json` key (nested keys are dotted: `permissions.defaultMode`).
    Setting,
    /// An environment variable (in a settings file's `env` block, or the launch environment).
    Env,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub values: Vec<String>,
    pub meaning: String,
    /// How the docs describe the value, when the type alone doesn't say it.
    #[serde(default)]
    pub shape: Option<String>,
    /// Where Claude Code honors it: user, project, local, managed, session (`--settings`), shell, global.
    pub scopes: Vec<String>,
    /// Narrower places for particular values (`permissions.defaultMode = "auto"` is ignored in project files).
    #[serde(default)]
    pub value_scopes: BTreeMap<String, Vec<String>>,
    #[serde(default = "stable")]
    pub stability: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// False for entries taken from a summary of the docs rather than the docs themselves.
    #[serde(default = "yes")]
    pub checked: bool,
    #[serde(skip)]
    pub kind: Option<Kind>,
}

fn stable() -> String {
    "stable".into()
}
fn yes() -> bool {
    true
}

impl Entry {
    pub fn kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Setting)
    }
    pub fn experimental(&self) -> bool {
        self.stability == "experimental"
    }
    pub fn deprecated(&self) -> bool {
        self.stability == "deprecated"
    }
    /// The type as a person reads it: `bool`, `one of low, medium, high`, `"1" to turn on`, ...
    pub fn type_label(&self) -> String {
        match self.ty.as_str() {
            "enum" if !self.values.is_empty() => format!("one of {}", self.values.join(", ")),
            "flag" => "\"1\" to turn on".into(),
            "removed" => "removed".into(),
            _ => self.shape.clone().unwrap_or_else(|| self.ty.clone()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Cond {
    key: String,
    #[serde(default)]
    is: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct Rule {
    when: Vec<Cond>,
    #[serde(default)]
    unless: Vec<Cond>,
    level: String,
    message: String,
}

#[derive(Deserialize)]
struct Data {
    setting: Vec<Entry>,
    env: Vec<Entry>,
    #[serde(default)]
    rule: Vec<Rule>,
}

struct Catalog {
    entries: Vec<Entry>,
    rules: Vec<Rule>,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let data: Data = toml::from_str(DATA).expect("catalog/claude-code.toml is valid");
    let mut entries = Vec::with_capacity(data.setting.len() + data.env.len());
    for (kind, list) in [(Kind::Setting, data.setting), (Kind::Env, data.env)] {
        entries.extend(list.into_iter().map(|mut e| {
            e.kind = Some(kind);
            e
        }));
    }
    Catalog {
        entries,
        rules: data.rule,
    }
});

/// A settings key (`permissions.defaultMode`) or an environment variable (`CLAUDE_CODE_EXPERIMENTAL_CODE_MODE`).
pub fn lookup(name: &str) -> Option<&'static Entry> {
    CATALOG.entries.iter().find(|e| e.name == name)
}

/// Known names within two edits of `name` (typos: `efortLevel` → `effortLevel`), closest first.
pub fn near(name: &str) -> Vec<&'static Entry> {
    fn distance(a: &str, b: &str) -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.iter().enumerate() {
            let mut cur = vec![i + 1];
            for (j, cb) in b.iter().enumerate() {
                let sub = prev[j] + usize::from(!ca.eq_ignore_ascii_case(cb));
                cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    let mut hits: Vec<(usize, &Entry)> = CATALOG
        .entries
        .iter()
        .map(|e| (distance(name, &e.name), e))
        .filter(|(d, _)| *d <= 2)
        .collect();
    hits.sort_by_key(|(d, e)| (*d, e.name.len()));
    hits.into_iter().map(|(_, e)| e).collect()
}

/// What to list.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filter {
    pub experimental: bool,
    pub kind: Option<Kind>,
    /// Include deprecated and removed keys.
    pub deprecated: bool,
}

/// Entries whose name or meaning contains every word of `query` (case-insensitive), names matching first.
pub fn search(query: &str, filter: Filter) -> Vec<&'static Entry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut hits: Vec<(bool, &Entry)> = CATALOG
        .entries
        .iter()
        .filter(|e| !filter.experimental || e.experimental())
        .filter(|e| filter.deprecated || !e.deprecated())
        .filter(|e| filter.kind.is_none_or(|k| e.kind() == k))
        .filter_map(|e| {
            let name = e.name.to_lowercase();
            let text = format!("{name} {}", e.meaning.to_lowercase());
            words
                .iter()
                .all(|w| text.contains(w.as_str()))
                .then(|| (words.iter().all(|w| name.contains(w.as_str())), e))
        })
        .collect();
    hits.sort_by(|(a, x), (b, y)| {
        b.cmp(a)
            .then(x.kind().cmp(&y.kind()))
            .then(x.name.cmp(&y.name))
    });
    hits.into_iter().map(|(_, e)| e).collect()
}

/// Where a settings value will be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `--settings` on the command line: how seatwise passes session presets and profiles.
    Session,
    /// `.claude/settings.local.json`: where seatwise writes `apply = "project"` presets.
    Local,
    /// `~/.claude/settings.json` (a seat's `settings.json`).
    User,
}

impl Target {
    fn scope(self) -> &'static str {
        match self {
            Target::Session => "session",
            Target::Local => "local",
            Target::User => "user",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Target::Session => "session settings (--settings)",
            Target::Local => "the project's .claude/settings.local.json",
            Target::User => "the seat's settings.json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub level: Level,
    pub key: String,
    pub message: String,
}

fn problem(level: Level, key: &str, message: impl Into<String>) -> Problem {
    Problem {
        level,
        key: key.to_string(),
        message: message.into(),
    }
}

/// Every key set in `settings`, flattened: dotted settings keys the catalog knows (descending into objects it
/// describes as objects), and each variable of the `env` block. `hooks` belong to seatwise and Claude Code.
fn flatten(settings: &Value) -> Vec<(Kind, String, Value)> {
    fn walk(prefix: &str, v: &Value, out: &mut Vec<(Kind, String, Value)>) {
        let Some(map) = v.as_object() else { return };
        for (k, v) in map {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            if prefix.is_empty() && k == "hooks" {
                continue;
            }
            if prefix.is_empty() && k == "env" {
                for (name, value) in v.as_object().into_iter().flatten() {
                    out.push((Kind::Env, name.clone(), value.clone()));
                }
                continue;
            }
            match lookup(&path) {
                Some(e) if e.ty == "object" && v.is_object() && has_children(&path) => {
                    walk(&path, v, out)
                }
                Some(_) => out.push((Kind::Setting, path, v.clone())),
                // Inside an object the catalog describes only as a whole (`modelSettings.<model>`): fine.
                None if !prefix.is_empty() => {}
                None => out.push((Kind::Setting, path, v.clone())),
            }
        }
    }
    let mut out = Vec::new();
    walk("", settings, &mut out);
    out
}

fn has_children(path: &str) -> bool {
    let prefix = format!("{path}.");
    CATALOG.entries.iter().any(|e| e.name.starts_with(&prefix))
}

/// A value as text, the way the catalog's values and `value_scopes` spell it.
fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn type_ok(e: &Entry, v: &Value) -> bool {
    let env = e.kind() == Kind::Env;
    match e.ty.as_str() {
        "bool" if env => matches!(as_text(v).as_str(), "1" | "0" | "true" | "false"),
        "bool" => v.is_boolean(),
        "flag" => matches!(as_text(v).as_str(), "1" | "0" | "true" | "false"),
        "number" if env => as_text(v).trim().parse::<f64>().is_ok(),
        "number" => v.is_number(),
        "enum" => {
            e.values.is_empty()
                || e.values.contains(&as_text(v))
                || (e.name == "model" || e.name.ends_with("Model"))
        }
        "list" => v.is_array() || (env && v.is_string()),
        "object" => v.is_object() || (env && v.is_string()),
        "string" => v.is_string() || env,
        _ => true,
    }
}

fn matches(cond: &Cond, set: &BTreeMap<String, Value>) -> bool {
    let Some(v) = set.get(&cond.key) else {
        return false;
    };
    match &cond.is {
        None => !matches!(as_text(v).as_str(), "" | "0"),
        Some(toml::Value::Boolean(b)) => v.as_bool() == Some(*b) || as_text(v) == b.to_string(),
        Some(toml::Value::String(s)) => as_text(v) == *s,
        Some(other) => as_text(v) == other.to_string(),
    }
}

/// Problems with `settings` applied at `target`: unknown keys, wrong types or values, keys Claude Code ignores
/// there, missing requirements, deprecated keys, experimental keys (info), and documented combinations.
pub fn check(settings: &Value, target: Target) -> Vec<Problem> {
    let mut out = Vec::new();
    let set = flatten(settings);
    let by_name: BTreeMap<String, Value> =
        set.iter().map(|(_, k, v)| (k.clone(), v.clone())).collect();
    for (kind, key, value) in &set {
        let Some(e) = lookup(key).filter(|e| e.kind() == *kind) else {
            let what = match kind {
                Kind::Env => "environment variable",
                Kind::Setting => "setting",
            };
            let hint = match near(key).first() {
                Some(e) => format!("did you mean {}?", e.name),
                None => format!("check the spelling (catalog checked {CHECKED})"),
            };
            out.push(problem(
                Level::Warn,
                key,
                format!("{key} is not a Claude Code {what} seatwise knows: {hint}"),
            ));
            continue;
        };
        if e.ty == "removed" || e.deprecated() {
            out.push(problem(
                Level::Warn,
                key,
                format!("{key} is deprecated or removed: {}", e.meaning),
            ));
            continue;
        }
        if !type_ok(e, value) {
            out.push(problem(
                Level::Error,
                key,
                format!("{key} = {value} is not valid: expected {}", e.type_label()),
            ));
            continue;
        }
        let scopes = e.value_scopes.get(&as_text(value)).unwrap_or(&e.scopes);
        // seatwise exports a session's launch-only variables itself (see `launch`), so they work there.
        let exported = target == Target::Session
            && e.kind() == Kind::Env
            && scopes.iter().any(|s| s == "shell");
        if !exported && !scopes.iter().any(|s| s == target.scope()) {
            let wher = scopes.join(", ");
            out.push(problem(
                Level::Error,
                key,
                format!(
                    "Claude Code ignores {key} = {} in {}; it is honored only in: {wher}",
                    as_text(value),
                    target.label()
                ),
            ));
        }
        for r in &e.requires {
            let (name, want) = r
                .split_once('=')
                .map_or((r.as_str(), None), |(n, v)| (n, Some(v)));
            let have = by_name.get(name).map(as_text);
            let ok = match want {
                Some(w) => have.as_deref() == Some(w),
                None => have.is_some_and(|h| !h.is_empty() && h != "0"),
            };
            if !ok {
                out.push(problem(
                    Level::Warn,
                    key,
                    format!("{key} needs {r} as well"),
                ));
            }
        }
        if e.experimental() {
            out.push(problem(
                Level::Info,
                key,
                format!("{key} is experimental: Claude Code may change or drop it"),
            ));
        }
    }
    for rule in &CATALOG.rules {
        if rule.when.iter().all(|c| matches(c, &by_name))
            && !rule.unless.iter().any(|c| matches(c, &by_name))
        {
            let level = match rule.level.as_str() {
                "error" => Level::Error,
                "info" => Level::Info,
                _ => Level::Warn,
            };
            out.push(problem(level, &rule.when[0].key, rule.message.clone()));
        }
    }
    out.sort_by(|a, b| a.level.cmp(&b.level).then(a.key.cmp(&b.key)));
    out
}

/// Problems as lines: `error  key: message`.
pub fn report(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|p| {
            let tag = match p.level {
                Level::Error => "error",
                Level::Warn => "warn ",
                Level::Info => "info ",
            };
            format!("  {tag}  {}\n", p.message)
        })
        .collect()
}

/// Every settings entry as a person reads it, for `seat settings show`.
pub fn describe(e: &Entry) -> String {
    let kind = match e.kind() {
        Kind::Setting => "setting (settings.json)",
        Kind::Env => "environment variable (a settings file's env block, or the shell)",
    };
    let mut s = format!("{}\n  {}\n\n", e.name, e.meaning);
    s += &format!("  kind       {kind}\n");
    s += &format!("  value      {}\n", e.type_label());
    s += &format!("  honored in {}\n", e.scopes.join(", "));
    for (v, sc) in &e.value_scopes {
        s += &format!("             {v}: only in {}\n", sc.join(", "));
    }
    if e.stability != "stable" {
        s += &format!("  stability  {}\n", e.stability);
    }
    if !e.requires.is_empty() {
        s += &format!("  needs      {}\n", e.requires.join(", "));
    }
    if let Some(v) = &e.since {
        s += &format!("  since      Claude Code {v}\n");
    }
    if let Some(n) = &e.note {
        s += &format!("\n  {n}\n");
    }
    if !e.checked {
        s += "\n  (taken from a summary of the docs; check it against them)\n";
    }
    let example = match e.ty.as_str() {
        "flag" => "\"1\"".to_string(),
        "bool" if e.kind() == Kind::Env => "\"1\"".to_string(),
        "bool" => "true".to_string(),
        "enum" if !e.values.is_empty() => format!("\"{}\"", e.values[0]),
        _ => "...".to_string(),
    };
    let how = match (e.kind(), e.scopes.iter().any(|s| s == "session")) {
        (Kind::Env, true) => format!("[preset.<name>.settings.env]\n{} = {example}", e.name),
        (Kind::Env, false) => format!("export {}=... before starting the session", e.name),
        (Kind::Setting, _) => match e.name.rsplit_once('.') {
            Some((table, key)) => format!("[preset.<name>.settings.{table}]\n{key} = {example}"),
            None => format!("[preset.<name>.settings]\n{} = {example}", e.name),
        },
    };
    s += &format!(
        "\n  in a seatwise preset:\n{}\n",
        how.lines()
            .map(|l| format!("    {l}\n"))
            .collect::<String>()
            .trim_end()
    );
    s
}

/// Only the problems that make settings wrong (errors), for edits that must stay valid.
pub fn errors(settings: &Value, target: Target) -> Vec<Problem> {
    check(settings, target)
        .into_iter()
        .filter(|p| p.level == Level::Error)
        .collect()
}

/// Environment variables Claude Code reads only from the launch environment: seatwise exports these itself,
/// since a settings `env` block can't carry them.
pub fn launch_only(name: &str) -> bool {
    lookup(name).is_some_and(|e| e.kind() == Kind::Env && e.scopes == ["shell"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_data_loads_and_knows_code_mode() {
        let e = lookup("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE").expect("in the catalog");
        assert!(e.experimental());
        assert_eq!(e.kind(), Kind::Env);
        assert!(CATALOG.entries.len() > 600);
        assert!(!search("code mode", Filter::default()).is_empty());
        assert!(
            search(
                "",
                Filter {
                    experimental: true,
                    ..Filter::default()
                }
            )
            .iter()
            .all(|e| e.experimental())
        );
    }

    #[test]
    fn code_mode_works_for_sessions_but_not_project_files() {
        let s = json!({ "env": { "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE": "1" } });
        assert!(errors(&s, Target::Session).is_empty());
        let e = errors(&s, Target::Local);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].message.contains("ignores"), "{e:?}");
    }

    #[test]
    fn types_values_and_value_scopes() {
        let bad = json!({ "effortLevel": "hihg", "alwaysThinkingEnabled": "yes" });
        assert_eq!(errors(&bad, Target::Session).len(), 2);
        let auto = json!({ "permissions": { "defaultMode": "auto" } });
        assert!(errors(&auto, Target::Session).is_empty());
        assert_eq!(errors(&auto, Target::Local).len(), 1);
        let plan = json!({ "permissions": { "defaultMode": "plan", "allow": ["Bash(ls)"] } });
        assert!(
            check(&plan, Target::Local).is_empty(),
            "{:?}",
            check(&plan, Target::Local)
        );
    }

    #[test]
    fn unknown_keys_warn_and_rules_fire() {
        let s = json!({ "efortLevel": "low", "alwaysThinkingEnabled": false, "env": { "MAX_THINKING_TOKENS": "2048" } });
        let p = check(&s, Target::Session);
        assert!(
            p.iter()
                .any(|p| p.key == "efortLevel" && p.message.contains("did you mean effortLevel")),
            "{p:?}"
        );
        assert!(
            p.iter().any(|p| p.message.contains("turns thinking on")),
            "{p:?}"
        );
        assert!(p.iter().all(|p| p.level != Level::Error), "{p:?}");
    }

    #[test]
    fn built_in_profiles_and_presets_are_clean() {
        for (name, _) in crate::plan::BUILTIN {
            let s = crate::plan::builtin(name).unwrap();
            let p = check(&s, Target::Session);
            assert!(p.iter().all(|p| p.level == Level::Info), "{name}: {p:?}");
        }
        let cfg: crate::config::Config = toml::from_str("").unwrap();
        for p in crate::preset::all(&cfg) {
            let s = crate::preset::settings(&cfg, &p);
            for t in [Target::Session, Target::Local] {
                assert!(check(&s, t).is_empty(), "{}: {:?}", p.name, check(&s, t));
            }
        }
    }
}
