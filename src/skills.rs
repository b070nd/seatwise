//! The seatwise skillset: skills that let a session (in the desktop app or the CLI) set up the command line next to
//! it, open a terminal for the project, report the seat's status and usage, and work from the seat's home repo.
//!
//! Claude Code seats get them as the `seatwise` plugin, so they are invoked as `/seatwise:hybrid`, `/seatwise:terminal`,
//! `/seatwise:status` and `/seatwise:home`. The plugin lives in a small marketplace inside the seat's config folder
//! (`seatwise-plugins/`), is listed in the seat's settings.json (`extraKnownMarketplaces`, `enabledPlugins`) and
//! registered with Claude Code's own `claude plugin` commands. Claude Code reads it in place, so a reinstall takes
//! effect at the next session (or `/reload-plugins`). Codex has no plugins: its seats get loose `seat-<name>` skills.
//!
//! [`state`] says what a seat has; [`install`] brings it up to date, removing the loose skills of seatwise 0.3 and
//! earlier from Claude seats.

use crate::config::{Agent, Config, Seat};
use crate::paths;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The plugin, and so the namespace of its skills: `/seatwise:<skill>`.
pub const PLUGIN: &str = "seatwise";
/// The marketplace seatwise writes into each Claude seat.
pub const MARKETPLACE: &str = "seatwise";
/// Plugins earlier versions shipped the skills as (0.4.0: `b070nd`); [`install`] removes them.
const RETIRED: [&str; 1] = ["b070nd"];

pub const SKILLS: [(&str, &str); 4] = [
    ("hybrid", include_str!("../skills/hybrid/SKILL.md")),
    ("terminal", include_str!("../skills/terminal/SKILL.md")),
    ("status", include_str!("../skills/status/SKILL.md")),
    ("home", include_str!("../skills/home/SKILL.md")),
];

/// `seatwise@seatwise`: how Claude Code names the installed plugin.
pub fn plugin_id() -> String {
    format!("{PLUGIN}@{MARKETPLACE}")
}

/// What a user types: `/seatwise:hybrid` on a Claude seat, `$seat-hybrid` on a Codex one.
pub fn invocation(agent: Agent, name: &str) -> String {
    match agent {
        Agent::Claude => format!("/{PLUGIN}:{name}"),
        Agent::Codex => format!("$seat-{name}"),
    }
}

/// The skill's name in its frontmatter: Claude Code prefixes the plugin, Codex has no namespace.
fn skill_name(agent: Agent, name: &str) -> String {
    match agent {
        Agent::Claude => name.to_owned(),
        Agent::Codex => format!("seat-{name}"),
    }
}

/// A skill as installed: its name and the path of this `seat` binary filled in.
pub fn render(agent: Agent, name: &str, text: &str, exe: &Path) -> String {
    text.replace("{{name}}", &skill_name(agent, name))
        .replace("{{seat}}", &exe.display().to_string())
}

/// The folder that holds the skill folders.
pub fn target(seat: &Seat) -> PathBuf {
    match seat.agent {
        Agent::Claude => marketplace_dir(seat)
            .join("plugins")
            .join(PLUGIN)
            .join("skills"),
        // Codex has no per-account skills folder: the seat's home repo (`.agents/skills`), or else
        // `~/.agents/skills`, which every Codex seat shares.
        Agent::Codex => match seat.home() {
            Some(h) => h.join(".agents").join("skills"),
            None => paths::home().join(".agents").join("skills"),
        },
    }
}

fn marketplace_dir(seat: &Seat) -> PathBuf {
    seat.config_dir().join("seatwise-plugins")
}

/// Every file seatwise writes for a seat, with its content.
fn files(seat: &Seat, exe: &Path) -> Vec<(PathBuf, String)> {
    let skills = target(seat);
    let mut out: Vec<(PathBuf, String)> = SKILLS
        .iter()
        .map(|(name, text)| {
            (
                skills.join(skill_name(seat.agent, name)).join("SKILL.md"),
                render(seat.agent, name, text, exe),
            )
        })
        .collect();
    if seat.agent == Agent::Claude {
        let root = marketplace_dir(seat);
        let version = env!("CARGO_PKG_VERSION");
        let marketplace = json!({
            "name": MARKETPLACE,
            "owner": { "name": "seatwise" },
            "plugins": [{
                "name": PLUGIN,
                "source": format!("./plugins/{PLUGIN}"),
                "description": "seatwise: hybrid desktop + CLI work, terminals, seat status and home repos",
            }],
        });
        let plugin = json!({
            "name": PLUGIN,
            "version": version,
            "description": "seatwise skills: /seatwise:hybrid, /seatwise:terminal, /seatwise:status, /seatwise:home",
            "homepage": "https://github.com/b070nd/seatwise",
            "license": "MIT",
        });
        out.push((
            root.join(".claude-plugin").join("marketplace.json"),
            pretty(&marketplace),
        ));
        out.push((
            root.join("plugins")
                .join(PLUGIN)
                .join(".claude-plugin")
                .join("plugin.json"),
            pretty(&plugin),
        ));
    }
    out
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("json") + "\n"
}

/// What a seat has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// Skills missing or different from this binary's (by name).
    pub outdated: Vec<String>,
    /// Claude: settings.json lists the marketplace and enables the plugin. Codex: always true.
    pub listed: bool,
    /// Claude: Claude Code has the plugin installed (`claude plugin install` ran). Codex: always true.
    pub registered: bool,
    /// Loose skills from seatwise 0.3 and earlier, still in a Claude seat's `skills/` folder.
    pub legacy: Vec<PathBuf>,
    /// Plugins of earlier versions still enabled, installed or on disk (by id).
    pub retired: Vec<String>,
}

impl State {
    pub fn current(&self) -> bool {
        self.outdated.is_empty()
            && self.listed
            && self.registered
            && self.legacy.is_empty()
            && self.retired.is_empty()
    }

    /// One line for `seat hybrid` and `seat doctor`.
    pub fn summary(&self) -> String {
        if self.current() {
            return "up to date".into();
        }
        let mut parts = Vec::new();
        if !self.outdated.is_empty() {
            parts.push(format!("missing or outdated: {}", self.outdated.join(", ")));
        }
        if !self.listed {
            parts.push(format!("{} not enabled in settings.json", plugin_id()));
        }
        if self.listed && !self.registered {
            parts.push("plugin not registered with Claude Code yet".into());
        }
        if !self.retired.is_empty() {
            parts.push(format!("old plugin to remove: {}", self.retired.join(", ")));
        }
        if !self.legacy.is_empty() {
            parts.push(format!(
                "{} old loose skill(s) to remove",
                self.legacy.len()
            ));
        }
        parts.join("; ")
    }
}

pub fn state(seat: &Seat, exe: &Path) -> State {
    let wanted = files(seat, exe);
    let skills = target(seat);
    let mut outdated: Vec<String> = Vec::new();
    for (path, text) in &wanted {
        if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        let name = if path.starts_with(&skills) {
            path.parent()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            "plugin manifest".into()
        };
        if !outdated.contains(&name) {
            outdated.push(name);
        }
    }
    match seat.agent {
        Agent::Codex => State {
            outdated,
            listed: true,
            registered: true,
            legacy: Vec::new(),
            retired: Vec::new(),
        },
        Agent::Claude => {
            let settings = read_json(&seat.hooks_file());
            State {
                outdated,
                listed: listed(&settings, seat),
                registered: installed_ids(seat).contains(&format!("\"{}\"", plugin_id())),
                legacy: legacy(seat),
                retired: retired(seat, &settings)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect(),
            }
        }
    }
}

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn installed_ids(seat: &Seat) -> String {
    read_json(
        &seat
            .config_dir()
            .join("plugins")
            .join("installed_plugins.json"),
    )
    .to_string()
}

/// Retired plugins this seat still has: enabled in settings.json, installed, or on disk.
fn retired(seat: &Seat, settings: &Value) -> Vec<(String, &'static str)> {
    let installed = installed_ids(seat);
    RETIRED
        .into_iter()
        .map(|old| (format!("{old}@{MARKETPLACE}"), old))
        .filter(|(id, old)| {
            !settings["enabledPlugins"][id.as_str()].is_null()
                || installed.contains(&format!("\"{id}\""))
                || marketplace_dir(seat).join("plugins").join(old).exists()
        })
        .collect()
}

fn listed(settings: &Value, seat: &Seat) -> bool {
    let dir = marketplace_dir(seat).display().to_string();
    settings["extraKnownMarketplaces"][MARKETPLACE]["source"]
        == json!({"source": "directory", "path": dir})
        && settings["enabledPlugins"][plugin_id()] == json!(true)
}

/// Loose skills seatwise 0.3 and earlier wrote into a Claude seat's `skills/`: a folder holding only a SKILL.md
/// whose frontmatter names it `seat-<skill>`. Anything else there is the user's and is left alone.
fn legacy(seat: &Seat) -> Vec<PathBuf> {
    let skills = seat.config_dir().join("skills");
    SKILLS
        .iter()
        .map(|(name, _)| skills.join(format!("seat-{name}")))
        .filter(|dir| {
            let only_skill = std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.file_name())
                        .eq([std::ffi::OsString::from("SKILL.md")])
                })
                .unwrap_or(false);
            only_skill
                && std::fs::read_to_string(dir.join("SKILL.md")).is_ok_and(|t| {
                    let name = dir.file_name().unwrap_or_default().to_string_lossy();
                    t.starts_with(&format!("---\nname: {name}\ndescription: "))
                })
        })
        .collect()
}

/// What [`install`] did.
#[derive(Debug, Default)]
pub struct Installed {
    /// The folder that holds the skill folders.
    pub dir: PathBuf,
    /// Old loose skills removed.
    pub removed: Vec<PathBuf>,
    /// Claude: `claude plugin` registered the plugin now (false: it registers at the next session start).
    pub registered: bool,
    /// Plugins of earlier versions removed (by id).
    pub retired: Vec<String>,
    /// Why registering (or removing an old plugin) failed, if it did.
    pub note: Option<String>,
}

/// Write the skills (and, for Claude, the plugin, its marketplace and the settings that enable it), remove old loose
/// skills, and register the plugin with Claude Code.
pub fn install(cfg: &Config, seat: &Seat, exe: &Path) -> Result<Installed> {
    install_with(cfg, seat, exe, claude)
}

fn install_with(
    cfg: &Config,
    seat: &Seat,
    exe: &Path,
    claude: impl Fn(&Seat, &[String]) -> Result<()>,
) -> Result<Installed> {
    for (path, text) in files(seat, exe) {
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    let mut done = Installed {
        dir: target(seat),
        registered: true,
        ..Installed::default()
    };
    if seat.agent == Agent::Codex {
        return Ok(done);
    }
    for dir in legacy(seat) {
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        done.removed.push(dir);
    }
    let path = seat.hooks_file();
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        Err(_) => json!({}),
    };
    let mut changed = false;
    let mut notes: Vec<String> = Vec::new();
    // Plugins of earlier versions: uninstall, disable, delete.
    let installed = installed_ids(seat);
    for (id, old) in retired(seat, &settings) {
        if installed.contains(&format!("\"{id}\""))
            && let Err(e) = claude(seat, &["plugin".into(), "uninstall".into(), id.clone()])
        {
            notes.push(format!("{e:#}"));
        }
        if let Some(enabled) = settings
            .get_mut("enabledPlugins")
            .and_then(Value::as_object_mut)
        {
            changed |= enabled.remove(&id).is_some();
        }
        let dir = marketplace_dir(seat).join("plugins").join(old);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        done.retired.push(id);
    }
    if !listed(&settings, seat) {
        changed = true;
        let obj = settings
            .as_object_mut()
            .with_context(|| format!("{} is not a JSON object", path.display()))?;
        let dir = marketplace_dir(seat).display().to_string();
        obj.entry("extraKnownMarketplaces")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("extraKnownMarketplaces is not an object")?
            .insert(
                MARKETPLACE.into(),
                json!({"source": {"source": "directory", "path": dir}}),
            );
        obj.entry("enabledPlugins")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("enabledPlugins is not an object")?
            .insert(plugin_id(), json!(true));
    }
    if changed {
        std::fs::write(&path, pretty(&settings))
            .with_context(|| format!("writing {}", path.display()))?;
        crate::sync::accept(seat, &cfg.track)?; // our own change is not drift
    }
    if !state(seat, exe).registered {
        match register(seat, &claude) {
            Ok(()) => done.registered = state(seat, exe).registered,
            Err(e) => {
                done.registered = false;
                notes.push(format!("{e:#}"));
            }
        }
    }
    if !notes.is_empty() {
        done.note = Some(notes.join("; "));
    }
    Ok(done)
}

/// Run `claude <args>` in the seat's own config folder.
fn claude(seat: &Seat, args: &[String]) -> Result<()> {
    let out = crate::launch::seat_command(seat)?
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .context("running claude")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "`claude {}` failed: {}",
            args.iter().take(2).cloned().collect::<Vec<_>>().join(" "),
            err.lines().last().unwrap_or("").trim()
        );
    }
    Ok(())
}

/// `claude plugin marketplace add` + `claude plugin install`, in the seat's own config folder.
fn register(seat: &Seat, claude: impl Fn(&Seat, &[String]) -> Result<()>) -> Result<()> {
    let dir = marketplace_dir(seat);
    let steps: [Vec<String>; 2] = [
        vec![
            "plugin".into(),
            "marketplace".into(),
            "add".into(),
            dir.display().to_string(),
        ],
        vec![
            "plugin".into(),
            "install".into(),
            plugin_id(),
            "--scope".into(),
            "user".into(),
        ],
    ];
    for args in steps {
        claude(seat, &args)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(dir: &Path, agent: &str) -> Seat {
        let cfg: Config = toml::from_str(&format!(
            "[[seat]]\nname = \"s\"\nagent = \"{agent}\"\nconfig_dir = '{}'\nhome = '{}'\n",
            dir.display(),
            dir.join("home").display()
        ))
        .unwrap();
        cfg.seats[0].clone()
    }

    #[test]
    fn skills_have_frontmatter_and_the_binary_path() {
        for (name, text) in SKILLS {
            assert!(
                text.starts_with("---\nname: {{name}}\ndescription: "),
                "{name}"
            );
            let out = render(Agent::Claude, name, text, Path::new("/x/seat"));
            assert!(out.starts_with(&format!("---\nname: {name}\n")), "{name}");
            assert!(out.contains("\"/x/seat\"") && !out.contains("{{"), "{name}");
            let codex = render(Agent::Codex, name, text, Path::new("/x/seat"));
            assert!(
                codex.starts_with(&format!("---\nname: seat-{name}\n")),
                "{name}"
            );
        }
    }

    #[test]
    fn a_claude_seat_gets_the_plugin_and_loses_the_old_loose_skills() {
        let t = tempfile::tempdir().unwrap();
        let s = seat(t.path(), "claude");
        let exe = Path::new("/x/seat");
        // seatwise 0.3 left these; the user's own skill next to them stays.
        let old = t.path().join("skills/seat-hybrid");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(
            old.join("SKILL.md"),
            "---\nname: seat-hybrid\ndescription: seat hybrid\n---\n",
        )
        .unwrap();
        let mine = t.path().join("skills/seat-status");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("SKILL.md"), "---\nname: seat-status\n---\n").unwrap();
        std::fs::write(mine.join("notes.md"), "mine").unwrap();
        std::fs::write(t.path().join("settings.json"), "{\"model\": \"opus\"}").unwrap();

        let before = state(&s, exe);
        assert_eq!(before.outdated.len(), 5, "{before:?}"); // four skills and the manifests
        assert!(!before.listed && before.legacy == vec![old.clone()]);

        // Registering fails (no claude here); the settings still enable it for the next start.
        let cfg: Config = toml::from_str("").unwrap();
        let done = install_with(&cfg, &s, exe, |_, _| anyhow::bail!("no claude")).unwrap();
        assert_eq!(done.removed, vec![old.clone()]);
        assert!(!done.registered && done.note.is_some());
        assert!(!old.exists() && mine.join("notes.md").exists());

        let after = state(&s, exe);
        assert!(
            after.outdated.is_empty() && after.listed && after.legacy.is_empty(),
            "{after:?}"
        );
        assert!(!after.registered);
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(t.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["model"], "opus");
        assert_eq!(settings["enabledPlugins"]["seatwise@seatwise"], true);
        let skill = t
            .path()
            .join("seatwise-plugins/plugins/seatwise/skills/hybrid/SKILL.md");
        assert!(
            std::fs::read_to_string(skill)
                .unwrap()
                .starts_with("---\nname: hybrid\n")
        );

        // What `claude plugin install` records.
        std::fs::create_dir_all(t.path().join("plugins")).unwrap();
        std::fs::write(
            t.path().join("plugins/installed_plugins.json"),
            "{\"plugins\": {\"seatwise@seatwise\": [{\"scope\": \"user\"}]}}",
        )
        .unwrap();
        assert!(state(&s, exe).current());
    }

    #[test]
    fn the_0_4_0_b070nd_plugin_is_uninstalled_and_disabled() {
        let t = tempfile::tempdir().unwrap();
        let s = seat(t.path(), "claude");
        let exe = Path::new("/x/seat");
        let old = t
            .path()
            .join("seatwise-plugins/plugins/b070nd/skills/hybrid");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(
            t.path().join("settings.json"),
            "{\"enabledPlugins\": {\"b070nd@seatwise\": true, \"other@x\": true}}",
        )
        .unwrap();
        std::fs::create_dir_all(t.path().join("plugins")).unwrap();
        let installed = t.path().join("plugins/installed_plugins.json");
        std::fs::write(&installed, "{\"plugins\": {\"b070nd@seatwise\": []}}").unwrap();
        assert_eq!(state(&s, exe).retired, ["b070nd@seatwise"]);

        // A stand-in for `claude plugin ...` that records what it is asked, the way Claude Code does.
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let cfg: Config = toml::from_str("").unwrap();
        let done = install_with(&cfg, &s, exe, |_, args| {
            calls.borrow_mut().push(args.join(" "));
            match args[1].as_str() {
                "uninstall" => std::fs::write(&installed, "{\"plugins\": {}}")?,
                "install" => {
                    std::fs::write(&installed, "{\"plugins\": {\"seatwise@seatwise\": []}}")?
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();
        let calls = calls.into_inner();
        assert_eq!(calls[0], "plugin uninstall b070nd@seatwise", "{calls:?}");
        assert!(
            calls
                .last()
                .unwrap()
                .starts_with("plugin install seatwise@seatwise")
        );
        assert_eq!(done.retired, ["b070nd@seatwise"]);
        assert!(!t.path().join("seatwise-plugins/plugins/b070nd").exists());
        let settings = std::fs::read_to_string(t.path().join("settings.json")).unwrap();
        assert!(
            !settings.contains("b070nd") && settings.contains("other@x"),
            "{settings}"
        );
        assert!(state(&s, exe).current(), "{:?}", state(&s, exe));
        assert_eq!(invocation(Agent::Claude, "hybrid"), "/seatwise:hybrid");
    }

    #[test]
    fn a_codex_seat_gets_loose_skills() {
        let t = tempfile::tempdir().unwrap();
        let s = seat(t.path(), "codex");
        assert!(!state(&s, Path::new("/x/seat")).current());
        let cfg: Config = toml::from_str("").unwrap();
        let done = install_with(&cfg, &s, Path::new("/x/seat"), |_, _| unreachable!()).unwrap();
        assert!(done.dir.join("seat-terminal/SKILL.md").exists());
        assert!(state(&s, Path::new("/x/seat")).current());
    }
}
