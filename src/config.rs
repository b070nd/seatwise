//! `config.toml`: the seats and which folders belong to them.

use crate::paths;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Folder owner meaning "every seat may start here" (a shared start folder).
pub const ANY: &str = "any";
/// Folder owner meaning "no seat may start in or touch this" (unless a longer entry assigns a subfolder).
pub const NONE: &str = "none";
const RESERVED: [&str; 3] = [ANY, NONE, "auto"];

/// How hard seatwise holds a seat to its folders and org.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Enforcement {
    /// No checks; folders only pick the seat. No hooks are installed.
    #[default]
    Off,
    /// Checks run and warn (the tool hook asks before touching another seat's folder), nothing is refused.
    Warn,
    /// Refuse to start, block prompts and tool calls (the air gap).
    Strict,
}

/// Where a seat's links (including `/login`) go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LinkSetting {
    /// `"system"` (default browser) or `"copy"` (clipboard + notification).
    Named(String),
    /// A command; `{url}` is replaced by the link, or the link is appended. E.g. a browser with a profile flag.
    Command(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link<'a> {
    System,
    Copy,
    Command(&'a [String]),
}

/// Options for `seat shell-init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ShellOptions {
    /// Also define plain `claude`, picking the seat from the current folder.
    pub override_claude: bool,
    /// Commands are named `<prefix><seat>` (Claude Code seats).
    pub prefix: String,
    /// The same for Codex seats.
    pub codex_prefix: String,
    /// Desktop app commands are named `<desktop_prefix><seat>` (macOS, Claude Code seats).
    pub desktop_prefix: String,
}

/// A seat's desktop app (`[seat.desktop]`). Every field is optional: by default a seat runs the installed app with
/// its own data folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    /// The app bundle. Default: `/Applications/Claude.app`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// The app's data folder (its own login and settings). `"default"` is the app's usual folder; default:
    /// `~/Library/Application Support/Claude-<seat>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<String>,
}

/// Which coding agent a seat runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    /// Claude Code (`claude`, `CLAUDE_CONFIG_DIR`).
    #[default]
    Claude,
    /// OpenAI Codex CLI, for ChatGPT plans (`codex`, `CODEX_HOME`).
    Codex,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
    /// The command it is started with.
    pub fn binary(self) -> &'static str {
        self.as_str()
    }
    /// The variable that points it at a config folder.
    pub fn home_var(self) -> &'static str {
        match self {
            Agent::Claude => "CLAUDE_CONFIG_DIR",
            Agent::Codex => "CODEX_HOME",
        }
    }
    /// Its default config folder name in the home folder.
    pub fn default_dir(self) -> &'static str {
        match self {
            Agent::Claude => ".claude",
            Agent::Codex => ".codex",
        }
    }
}

/// `seat term`: how a new terminal window is opened.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TerminalOptions {
    /// Usage profile applied to sessions opened with `seat term` (session only, not written to settings), e.g.
    /// "experimental". Unset: none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// macOS: the terminal app (default "Terminal"; e.g. "iTerm").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Any OS: your own command; `{dir}` and `{cmd}` (a shell command line) are filled in,
    /// e.g. `["kitty", "--directory", "{dir}", "sh", "-c", "{cmd}"]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
}

impl Enforcement {
    pub fn as_str(self) -> &'static str {
        match self {
            Enforcement::Off => "off",
            Enforcement::Warn => "warn",
            Enforcement::Strict => "strict",
        }
    }
}

/// Items `share` must never name: they hold a seat's login, history or guard hooks.
pub const NOT_SHAREABLE: [&str; 8] = [
    ".claude.json",
    ".credentials.json",
    "settings.json",
    "settings.local.json",
    "projects",
    "sessions",
    "history.jsonl",
    "todos",
];

fn is_claude(a: &Agent) -> bool {
    *a == Agent::Claude
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// Which agents are in use. A turned-off agent's seats behave as disabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Agents {
    pub claude: bool,
    pub codex: bool,
}

impl Default for Agents {
    fn default() -> Self {
        Self {
            claude: true,
            codex: true,
        }
    }
}

impl Agents {
    pub fn on(&self, agent: Agent) -> bool {
        match agent {
            Agent::Claude => self.claude,
            Agent::Codex => self.codex,
        }
    }
}

/// How a folder group's preset reaches its sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Apply {
    /// Passed as `--settings` when seatwise starts the session; nothing on disk changes.
    #[default]
    Session,
    /// Written into the project's `.claude/settings.local.json` on start, so a plain `claude` gets it too.
    Project,
}

impl Apply {
    pub fn as_str(self) -> &'static str {
        match self {
            Apply::Session => "session",
            Apply::Project => "project",
        }
    }
}

/// A use-case preset: usage profiles combined in order, then its own settings on top.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub settings: toml::Table,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

/// Folders that share a preset (one project, or a folder of projects). The longest matching path wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub paths: Vec<String>,
    /// A preset name, or "none" for no preset here (a project opting out of its parent's group).
    pub preset: String,
    /// Overrides the global `preset_apply`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply: Option<Apply>,
    /// Claude Code setting sources to load here (`--setting-sources`): any of "user", "project", "local".
    /// E.g. `["user"]` ignores the project's own settings files. Unset: all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// A disabled group is skipped; its folders fall back to a parent group.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

/// The sentinel: deterministic checks for settings that cost more than they should.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SentinelOptions {
    /// Check when a seat starts (at most once per `interval`). `seat sentinel` works either way.
    pub enabled: bool,
    /// "30m", "6h", "1d", ...
    pub interval: String,
    /// Also show a desktop notification when there are findings.
    pub notify: bool,
    /// Rules to silence: "rule" everywhere, or "rule@<path>" for one project.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
}

impl Default for SentinelOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: "24h".into(),
            notify: true,
            ignore: vec![],
        }
    }
}

/// `30m`, `6h`, `1d`, `90s` → seconds.
pub fn parse_interval(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = num.parse().ok()?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return None,
    };
    (n > 0).then_some(n * mult)
}

pub const SOURCES: [&str; 3] = ["user", "project", "local"];

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            override_claude: false,
            prefix: "claude-".into(),
            codex_prefix: "codex-".into(),
            desktop_prefix: "claude-desktop-".into(),
        }
    }
}

/// Bring a config written for an older format up to [`CONFIG_VERSION`] (in memory; the file is left as it is), and
/// refuse one written by a newer seatwise: its meaning can't be guessed, and reading it anyway could weaken the
/// isolation it asks for. Each future format change adds one step here.
pub fn migrate(raw: toml::Table) -> Result<toml::Table> {
    let version = match raw.get("version") {
        None => 1,
        Some(toml::Value::Integer(n)) if *n >= 1 => u32::try_from(*n).unwrap_or(u32::MAX),
        Some(other) => bail!("`version` must be a whole number from 1, not {other}"),
    };
    if version > CONFIG_VERSION {
        return Err(crate::error::UserError::new(format!(
            "this config is format version {version}, written by a newer seatwise; this one reads up to \
             {CONFIG_VERSION}"
        ))
        .resolution("Upgrade seatwise: cargo install seatwise --locked")
        .into());
    }
    // Version 1 is the first format: nothing to migrate yet.
    Ok(raw)
}

/// The config format this seatwise reads and writes. A config without `version` is version 1.
pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The config format (`CONFIG_VERSION`). Unset: 1. A newer one is refused, an older one migrated on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Default for every seat; a seat can override it.
    #[serde(default)]
    pub enforcement: Enforcement,
    /// Seat for `seat run auto` in folders that no seat owns. Unset: ask (on a terminal) or refuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_seat: Option<String>,
    /// Files in each seat's config folder to snapshot on every start (e.g. "settings.json"). Empty: off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track: Vec<String>,
    /// Tools the PreToolUse hook checks (a Claude Code matcher). Unset: the built-in list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_matcher: Option<String>,
    /// Where `share` items come from. Unset: `shared/` next to this config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_dir: Option<String>,
    #[serde(default)]
    pub shell: ShellOptions,
    #[serde(default)]
    pub terminal: TerminalOptions,
    #[serde(default)]
    pub agents: Agents,
    /// How folder groups' presets reach sessions unless a group says otherwise.
    #[serde(default)]
    pub preset_apply: Apply,
    #[serde(default)]
    pub sentinel: SentinelOptions,
    /// When Claude Code compacts the conversation, enforced on every Claude seat: a tier (`sharp`, `standard`,
    /// `lazy`) or a token count. A seat can override it. Unset: Claude Code's own setting stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<Compact>,
    /// Use-case presets; a name here replaces a built-in one.
    #[serde(
        default,
        rename = "preset",
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub presets: std::collections::BTreeMap<String, PresetDef>,
    #[serde(default, rename = "group", skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    /// Your own usage profiles (settings merged into a seat's settings.json); a name here replaces a built-in one.
    #[serde(
        default,
        rename = "profile",
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub profiles: std::collections::BTreeMap<String, toml::Table>,
    #[serde(default, rename = "seat")]
    pub seats: Vec<Seat>,
    #[serde(default, rename = "folder")]
    pub folders: Vec<Folder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seat {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// "claude" (default) or "codex".
    #[serde(default, skip_serializing_if = "is_claude")]
    pub agent: Agent,
    /// A disabled seat keeps its config but gets no shell command and can't be started.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// The seat's home repo (`seat home init`): its skills, AGENTS.md and pointers to this seat's other repos.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Claude Code config folder. `~/.claude` means Claude's default location (no `CLAUDE_CONFIG_DIR`).
    pub config_dir: String,
    /// Organization this seat must be signed into; prompts are blocked on any other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    /// Account email this seat must be signed in as (case-insensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// The plan pinned with the account (`seat pin`), e.g. "team_premium"; doctor reports a change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The usage profile last applied to this seat (`seat profile`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Items from the shared folder copied into this seat's config folder on every start,
    /// e.g. `["skills/review", "agents", "CLAUDE.md"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub share: Vec<String>,
    /// Overrides the global `enforcement` for this seat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<Enforcement>,
    /// Overrides the global `compact` for this seat; `"off"` leaves its settings alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<Compact>,
    /// `"system"` (default), `"copy"`, or a command such as `["firefox", "-P", "work", "{url}"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkSetting>,
    /// The desktop app for this seat (macOS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<Desktop>,
}

/// A compaction tier or token count: `compact = "sharp"`, `compact = "250k"` or `compact = 250000`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Compact {
    Tokens(u64),
    Text(String),
}

impl Compact {
    pub fn as_text(&self) -> String {
        match self {
            Compact::Tokens(n) => n.to_string(),
            Compact::Text(t) => t.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub path: String,
    /// A seat name, `"any"` or `"none"`.
    pub owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner<'a> {
    Seat(&'a str),
    Any,
    Nobody,
}

impl Folder {
    pub fn owner(&self) -> Owner<'_> {
        match self.owner.as_str() {
            ANY => Owner::Any,
            NONE => Owner::Nobody,
            seat => Owner::Seat(seat),
        }
    }
}

impl Default for Seat {
    fn default() -> Self {
        Self {
            name: String::new(),
            aliases: vec![],
            agent: Agent::Claude,
            enabled: true,
            home: None,
            config_dir: String::new(),
            org_uuid: None,
            org_name: None,
            email: None,
            plan: None,
            profile: None,
            share: vec![],
            enforcement: None,
            compact: None,
            link: None,
            desktop: None,
        }
    }
}

impl Seat {
    pub fn config_dir(&self) -> PathBuf {
        paths::expand(&self.config_dir)
    }

    /// Claude's default location: started without `CLAUDE_CONFIG_DIR`, state file in the home folder.
    pub fn is_default_location(&self) -> bool {
        self.config_dir() == paths::home().join(self.agent.default_dir())
    }

    pub fn home(&self) -> Option<PathBuf> {
        self.home.as_deref().map(paths::expand)
    }

    /// Where the agent reads hooks from.
    pub fn hooks_file(&self) -> PathBuf {
        self.config_dir().join(match self.agent {
            Agent::Claude => "settings.json",
            Agent::Codex => "hooks.json",
        })
    }

    pub fn link(&self) -> Link<'_> {
        match &self.link {
            None => Link::System,
            Some(LinkSetting::Named(n)) if n == "copy" => Link::Copy,
            Some(LinkSetting::Named(_)) => Link::System,
            Some(LinkSetting::Command(argv)) => Link::Command(argv),
        }
    }

    /// Claude Code's state file (holds the signed-in account).
    pub fn state_file(&self) -> PathBuf {
        if self.agent == Agent::Codex {
            self.config_dir().join("auth.json")
        } else if self.is_default_location() {
            paths::home().join(".claude.json")
        } else {
            self.config_dir().join(".claude.json")
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_from(&paths::config_file())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {} (see config.example.toml)", path.display()))?;
        Self::parse(&text).with_context(|| format!("reading {}", path.display()))
    }

    /// A config from its text: parsed, migrated to [`CONFIG_VERSION`] and checked.
    pub fn parse(text: &str) -> Result<Self> {
        let raw: toml::Table = toml::from_str(text).context("parsing")?;
        let cfg: Config = migrate(raw)?.try_into().context("parsing")?;
        cfg.validate().context("checking")?;
        Ok(cfg)
    }

    /// The config format of this config (1 when unset).
    pub fn format_version(&self) -> u32 {
        self.version.unwrap_or(1)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(c) = &self.compact {
            crate::compact::parse(&c.as_text())?;
        }
        for seat in &self.seats {
            if let Some(c) = &seat.compact {
                crate::compact::parse(&c.as_text())
                    .with_context(|| format!("seat '{}'", seat.name))?;
            }
        }
        let mut names = HashSet::new();
        for seat in &self.seats {
            for name in std::iter::once(&seat.name).chain(&seat.aliases) {
                if RESERVED.contains(&name.as_str()) {
                    bail!("seat name '{name}' is reserved");
                }
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    bail!("seat name '{name}' may only use letters, digits, '-' and '_'");
                }
                if !names.insert(name.as_str()) {
                    bail!("seat name '{name}' is used twice");
                }
            }
        }
        if let Some(d) = &self.default_seat
            && self.seat(d).is_none()
        {
            bail!("default_seat '{d}' is not a configured seat");
        }
        if ![
            &self.shell.prefix,
            &self.shell.codex_prefix,
            &self.shell.desktop_prefix,
        ]
        .iter()
        .flat_map(|p| p.chars())
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("shell.prefix may only use letters, digits, '-' and '_'");
        }
        for file in &self.track {
            if file.is_empty() || file == "." || file == ".." || file.contains(['/', '\\']) {
                bail!("track entry '{file}' must be a plain file name in the seat's config folder");
            }
        }
        let mut data_dirs = HashSet::new();
        for seat in &self.seats {
            if seat.desktop.is_some() && seat.agent != Agent::Claude {
                bail!(
                    "seat '{}': [seat.desktop] is for Claude Code seats",
                    seat.name
                );
            }
            if seat.agent == Agent::Claude {
                let key = match crate::desktop::data_dir(seat) {
                    Some(d) => crate::folders::key_string(&d),
                    None => "default".into(),
                };
                if seat.desktop.is_some() && !data_dirs.insert(key) {
                    bail!(
                        "seat '{}' uses the same desktop user_data_dir as another seat; every seat needs its own",
                        seat.name
                    );
                }
            }
        }
        let mut dirs = HashSet::new();
        for seat in &self.seats {
            let dir = crate::folders::key_string(&seat.config_dir());
            if !dirs.insert(dir) {
                bail!(
                    "seat '{}' uses the same config_dir as another seat; every seat needs its own",
                    seat.name
                );
            }
            for item in &seat.share {
                let path = Path::new(item);
                let first = path
                    .components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned());
                if item.is_empty()
                    || path.has_root()
                    || item.contains(':')
                    || path
                        .components()
                        .any(|c| !matches!(c, std::path::Component::Normal(_)))
                {
                    bail!(
                        "seat '{}': share entry '{item}' must be a relative path inside the shared folder",
                        seat.name
                    );
                }
                if first.is_some_and(|f| NOT_SHAREABLE.contains(&f.as_str())) {
                    bail!(
                        "seat '{}': '{item}' can't be shared (it holds the seat's login, history or guard hooks)",
                        seat.name
                    );
                }
            }
            match &seat.link {
                Some(LinkSetting::Named(n)) if n != "system" && n != "copy" => bail!(
                    "seat '{}': link must be \"system\", \"copy\" or a command list, not \"{n}\"",
                    seat.name
                ),
                Some(LinkSetting::Command(argv)) if argv.is_empty() => {
                    bail!("seat '{}': link command is empty", seat.name)
                }
                _ => {}
            }
        }
        if let Some(p) = &self.terminal.profile
            && !self.profiles.contains_key(p)
            && crate::plan::builtin(p).is_none()
        {
            bail!("terminal.profile: unknown profile '{p}'");
        }
        for (name, table) in &self.profiles {
            if table.contains_key("hooks") {
                bail!("profile '{name}' may not set hooks: they are the seats' guards");
            }
        }
        for seat in &self.seats {
            if let Some(p) = &seat.profile
                && !self.profiles.contains_key(p)
                && crate::plan::builtin(p).is_none()
            {
                bail!("seat '{}': unknown profile '{p}'", seat.name);
            }
        }
        if parse_interval(&self.sentinel.interval).is_none() {
            bail!(
                "sentinel.interval '{}' must look like 30m, 6h or 1d",
                self.sentinel.interval
            );
        }
        for (name, p) in &self.presets {
            if name == NONE || name.is_empty() {
                bail!("preset name '{name}' is reserved");
            }
            if p.settings.contains_key("hooks") {
                bail!("preset '{name}' may not set hooks: they are the seats' guards");
            }
            for prof in &p.profiles {
                if !self.profiles.contains_key(prof) && crate::plan::builtin(prof).is_none() {
                    bail!("preset '{name}': unknown profile '{prof}'");
                }
            }
        }
        let mut group_names = HashSet::new();
        let mut group_paths = HashSet::new();
        for g in &self.groups {
            if !group_names.insert(g.name.as_str()) {
                bail!("group name '{}' is used twice", g.name);
            }
            if g.paths.is_empty() {
                bail!("group '{}' has no paths", g.name);
            }
            if g.preset != NONE && crate::preset::get(self, &g.preset).is_none() {
                bail!("group '{}': unknown preset '{}'", g.name, g.preset);
            }
            for s in g.sources.iter().flatten() {
                if !SOURCES.contains(&s.as_str()) {
                    bail!(
                        "group '{}': setting source '{s}' must be user, project or local",
                        g.name
                    );
                }
            }
            for p in &g.paths {
                if !group_paths.insert(crate::folders::key_string(&paths::expand(p))) {
                    bail!("folder {p} is in two groups; a folder belongs to one group");
                }
            }
        }
        for folder in &self.folders {
            if let Owner::Seat(s) = folder.owner()
                && !self.seats.iter().any(|seat| seat.name == s)
            {
                bail!(
                    "folder {} is assigned to unknown seat '{s}' (use a seat's main name, \"any\" or \"none\")",
                    folder.path
                );
            }
        }
        Ok(())
    }

    /// Whether a seat may be used: it and its agent are enabled.
    pub fn is_enabled(&self, seat: &Seat) -> bool {
        seat.enabled && self.agents.on(seat.agent)
    }

    /// Find a seat by name or alias.
    /// The seat called `name` (or an alias), or an error that names the likely one.
    pub fn seat_named(&self, name: &str) -> Result<&Seat> {
        self.seat(name).ok_or_else(|| {
            let names: Vec<&str> = self
                .seats
                .iter()
                .flat_map(|s| {
                    std::iter::once(s.name.as_str()).chain(s.aliases.iter().map(String::as_str))
                })
                .collect();
            crate::error::UserError::not_found(crate::suggest::unknown("seat", name, &names)).into()
        })
    }

    pub fn seat(&self, name: &str) -> Option<&Seat> {
        self.seats
            .iter()
            .find(|s| s.name == name || s.aliases.iter().any(|a| a == name))
    }

    /// Where `share` items come from.
    pub fn shared_dir(&self) -> PathBuf {
        match &self.shared_dir {
            Some(d) => paths::expand(d),
            None => paths::config_file()
                .parent()
                .map(|p| p.join("shared"))
                .unwrap_or_else(|| paths::home().join("seatwise-shared")),
        }
    }

    /// The enforcement level that applies to `seat`.
    pub fn enforcement_of(&self, seat: &Seat) -> Enforcement {
        seat.enforcement.unwrap_or(self.enforcement)
    }

    pub fn seat_names(&self) -> String {
        self.seats
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Folders assigned to `seat`, home references expanded.
    pub fn folders_of(&self, seat: &str) -> Vec<PathBuf> {
        self.folders
            .iter()
            .filter(|f| f.owner() == Owner::Seat(seat))
            .map(|f| paths::expand(&f.path))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Config> {
        let cfg: Config = toml::from_str(s)?;
        cfg.validate()?;
        Ok(cfg)
    }

    #[test]
    fn parses_and_finds_aliases() {
        let cfg = parse(
            r#"
            [[seat]]
            name = "work"
            aliases = ["w"]
            config_dir = "~/.claude-work"
            [[folder]]
            path = "~/code"
            owner = "work"
            [[folder]]
            path = "~/start"
            owner = "any"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.seat("w").unwrap().name, "work");
        assert_eq!(cfg.folders[1].owner(), Owner::Any);
        assert_eq!(cfg.folders_of("work"), vec![paths::home().join("code")]);
        let work = cfg.seat("work").unwrap();
        assert_eq!(cfg.enforcement_of(work), Enforcement::Off, "off by default");
        assert_eq!(work.link(), Link::System, "system browser by default");
        assert!(!cfg.shell.override_claude);
        assert!(cfg.track.is_empty(), "snapshots are opt-in");
    }

    #[test]
    fn example_config_is_valid() {
        let cfg = parse(include_str!("../config.example.toml")).unwrap();
        assert_eq!(cfg.enforcement, Enforcement::Off);
        assert_eq!(cfg.seats.len(), 2);
    }

    #[test]
    fn options_and_overrides() {
        let cfg = parse(
            r#"
            enforcement = "warn"
            default_seat = "p"
            track = ["settings.json"]
            [shell]
            override_claude = true
            prefix = "cc-"
            [[seat]]
            name = "work"
            config_dir = "~/.claude-work"
            enforcement = "strict"
            link = ["firefox", "-P", "work", "{url}"]
            [[seat]]
            name = "personal"
            aliases = ["p"]
            config_dir = "~/.claude-personal"
            link = "copy"
            "#,
        )
        .unwrap();
        let (w, p) = (&cfg.seats[0], &cfg.seats[1]);
        assert_eq!(cfg.enforcement_of(w), Enforcement::Strict);
        assert_eq!(cfg.enforcement_of(p), Enforcement::Warn);
        assert!(matches!(w.link(), Link::Command(a) if a.len() == 4));
        assert_eq!(p.link(), Link::Copy);
        assert_eq!(cfg.shell.prefix, "cc-");
    }

    #[test]
    fn rejects_bad_configs() {
        let seat = |n: &str| format!("[[seat]]\nname = \"{n}\"\nconfig_dir = \"~/.c\"\n");
        assert!(parse(&seat("any")).is_err(), "reserved name");
        assert!(parse(&seat("a b")).is_err(), "bad characters");
        assert!(
            parse(&format!("{}{}", seat("x"), seat("x"))).is_err(),
            "duplicate"
        );
        assert!(
            parse(&format!(
                "{}[[folder]]\npath = \"~\"\nowner = \"y\"\n",
                seat("x")
            ))
            .is_err(),
            "unknown owner"
        );
        assert!(
            parse("[[seat]]\nname = \"x\"\nconfig_dir = \"~\"\ntypo = 1\n").is_err(),
            "unknown key"
        );
        assert!(
            parse(&format!("default_seat = \"q\"\n{}", seat("x"))).is_err(),
            "unknown default seat"
        );
        assert!(
            parse(&format!("track = [\"../x\"]\n{}", seat("x"))).is_err(),
            "track outside the config folder"
        );
        assert!(
            parse(&format!("{}link = \"chrome\"\n", seat("x"))).is_err(),
            "unknown link mode"
        );
        assert!(
            parse(&format!("{}{}", seat("x"), seat("y"))).is_err(),
            "two seats, one config_dir"
        );
        for bad in ["../x", "/abs", "settings.json", "projects/p", ""] {
            assert!(
                parse(&format!("{}share = [\"{bad}\"]\n", seat("x"))).is_err(),
                "share {bad}"
            );
        }
        assert!(
            parse(&format!(
                "{}share = [\"skills/review\", \"CLAUDE.md\"]\n",
                seat("x")
            ))
            .is_ok()
        );
    }
}
