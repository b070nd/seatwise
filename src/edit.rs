//! Changing `config.toml` from the command line (`seat add`, `link`, `unlink`, `pin`), keeping the user's
//! comments and layout. Every change is checked by loading the result before it is written.

use crate::config::Config;
use crate::folders::key_string;
use crate::paths;
use anyhow::{Context, Result, bail};
use std::path::Path;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

/// A JSON value as a TOML value (objects become inline tables; null becomes an empty string).
pub fn json_to_toml(v: &serde_json::Value) -> toml_edit::Value {
    use serde_json::Value as J;
    match v {
        J::Null => "".into(),
        J::Bool(b) => (*b).into(),
        J::Number(n) => n
            .as_i64()
            .map(toml_edit::Value::from)
            .unwrap_or_else(|| n.as_f64().unwrap_or_default().into()),
        J::String(s) => s.as_str().into(),
        J::Array(a) => a
            .iter()
            .map(json_to_toml)
            .collect::<toml_edit::Array>()
            .into(),
        J::Object(o) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, v) in o {
                t.insert(k, json_to_toml(v));
            }
            t.into()
        }
    }
}

#[derive(Clone)]
pub struct ConfigFile {
    path: std::path::PathBuf,
    doc: DocumentMut,
}

impl ConfigFile {
    /// The config file at its usual place; an empty one if it doesn't exist yet.
    pub fn open() -> Result<Self> {
        Self::open_at(&paths::config_file())
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let doc = text
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    /// The config as it would be after the pending changes.
    pub fn config(&self) -> Result<Config> {
        let cfg: Config = toml::from_str(&self.doc.to_string())?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Check, then write.
    pub fn save(&self) -> Result<()> {
        self.config()
            .context("the change would make the config invalid")?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, self.doc.to_string())
            .with_context(|| format!("writing {}", self.path.display()))
    }

    fn tables(&mut self, key: &str) -> &mut ArrayOfTables {
        if !self.doc.contains_array_of_tables(key) {
            self.doc
                .insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
        }
        self.doc[key].as_array_of_tables_mut().expect("just made")
    }

    fn seat_table(&mut self, name: &str) -> Option<&mut Table> {
        self.tables("seat").iter_mut().find(|t| {
            t.get("name").and_then(|v| v.as_str()) == Some(name)
                || t.get("aliases")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(name)))
        })
    }

    /// Every name a seat answers to, for "unknown seat" messages.
    fn seat_labels(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for t in self.tables("seat").iter() {
            out.extend(t.get("name").and_then(|v| v.as_str()).map(String::from));
            if let Some(a) = t.get("aliases").and_then(|v| v.as_array()) {
                out.extend(a.iter().filter_map(|x| x.as_str()).map(String::from));
            }
        }
        out
    }

    fn seat_table_checked(&mut self, name: &str) -> Result<&mut Table> {
        if self.seat_table(name).is_none() {
            let known = self.seat_labels();
            let known: Vec<&str> = known.iter().map(String::as_str).collect();
            return Err(crate::error::UserError::not_found(crate::suggest::unknown(
                "seat", name, &known,
            ))
            .into());
        }
        Ok(self.seat_table(name).expect("checked above"))
    }

    fn group_table_checked(&mut self, name: &str) -> Result<&mut Table> {
        if self.group_table(name).is_none() {
            let known: Vec<String> = self
                .tables("group")
                .iter()
                .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
                .collect();
            let known: Vec<&str> = known.iter().map(String::as_str).collect();
            return Err(crate::error::UserError::not_found(crate::suggest::unknown(
                "group", name, &known,
            ))
            .into());
        }
        Ok(self.group_table(name).expect("checked above"))
    }

    pub fn add_seat(&mut self, name: &str, config_dir: &str, aliases: &[String]) -> Result<()> {
        if self.seat_table(name).is_some() {
            bail!("seat '{name}' already exists");
        }
        let mut t = Table::new();
        t["name"] = value(name);
        if !aliases.is_empty() {
            t["aliases"] = value(aliases.iter().collect::<toml_edit::Array>());
        }
        t["config_dir"] = value(config_dir);
        self.tables("seat").push(t);
        Ok(())
    }

    /// Set a string field of a seat.
    pub fn set_seat(&mut self, seat: &str, key: &str, v: &str) -> Result<()> {
        let t = self.seat_table_checked(seat)?;
        t[key] = value(v);
        Ok(())
    }

    /// Set a field of a seat's `[seat.desktop]` table, creating the table after the seat's own keys.
    pub fn set_seat_desktop(&mut self, seat: &str, key: &str, v: &str) -> Result<()> {
        let t = self.seat_table_checked(seat)?;
        if !t.contains_key("desktop") {
            t.insert("desktop", Item::Table(Table::new()));
        }
        let d = match &mut t["desktop"] {
            Item::Table(d) => d,
            Item::Value(toml_edit::Value::InlineTable(d)) => {
                d.insert(key, v.into());
                return Ok(());
            }
            _ => bail!("seat '{seat}': `desktop` in the config is not a table"),
        };
        d[key] = value(v);
        Ok(())
    }

    /// Record the account (and plan) a seat must be signed into. `None` fields are left as they are.
    pub fn pin(
        &mut self,
        seat: &str,
        org_uuid: Option<&str>,
        org_name: Option<&str>,
        email: Option<&str>,
        plan: Option<&str>,
    ) -> Result<()> {
        let t = self.seat_table_checked(seat)?;
        for (k, v) in [
            ("org_uuid", org_uuid),
            ("org_name", org_name),
            ("email", email),
            ("plan", plan),
        ] {
            if let Some(v) = v {
                t[k] = value(v);
            }
        }
        Ok(())
    }

    /// The table at `path` (e.g. `["sentinel"]`, `["preset", "review"]`), created if missing. Intermediate tables
    /// are implicit, so `[preset.review]` is written without an empty `[preset]`.
    pub fn table_at(&mut self, path: &[&str]) -> Result<&mut Table> {
        let mut t: &mut Table = self.doc.as_table_mut();
        for (i, key) in path.iter().enumerate() {
            if !t.contains_key(key) {
                let mut new = Table::new();
                new.set_implicit(i + 1 < path.len());
                t.insert(key, Item::Table(new));
            }
            t = t[*key].as_table_mut().with_context(|| {
                format!("'{}' in the config is not a table", path[..=i].join("."))
            })?;
        }
        Ok(t)
    }

    /// Set a value at `path` + `key`; `None` removes it (back to the default).
    pub fn set(&mut self, path: &[&str], key: &str, v: Option<toml_edit::Value>) -> Result<()> {
        let t = if path.is_empty() {
            self.doc.as_table_mut()
        } else {
            self.table_at(path)?
        };
        match v {
            Some(v) => {
                t[key] = Item::Value(v);
            }
            None => {
                t.remove(key);
            }
        }
        Ok(())
    }

    /// Set any value of a seat; `None` removes it.
    pub fn set_seat_value(
        &mut self,
        seat: &str,
        key: &str,
        v: Option<toml_edit::Value>,
    ) -> Result<()> {
        let t = self.seat_table_checked(seat)?;
        match v {
            Some(v) => {
                t[key] = Item::Value(v);
            }
            None => {
                t.remove(key);
            }
        }
        Ok(())
    }

    /// `enabled = false` on a seat, or the key removed (enabled is the default).
    pub fn set_seat_enabled(&mut self, seat: &str, on: bool) -> Result<()> {
        let t = self.seat_table_checked(seat)?;
        if on {
            t.remove("enabled");
        } else {
            t["enabled"] = value(false);
        }
        Ok(())
    }

    /// Turn a preset on or off. A built-in one gets its own table (a copy of it), since a table replaces it.
    /// Write a preset's description, profiles and settings (`settings` as an inline-table value is spread into a
    /// `[preset.<name>.settings]` table). `enabled` and anything else in the table stay as they are.
    pub fn set_preset(
        &mut self,
        name: &str,
        description: &str,
        profiles: &[String],
        settings: &toml_edit::Value,
    ) -> Result<()> {
        let t = self.table_at(&["preset", name])?;
        if description.is_empty() {
            t.remove("description");
        } else {
            t["description"] = value(description);
        }
        if profiles.is_empty() {
            t.remove("profiles");
        } else {
            t["profiles"] = value(profiles.iter().collect::<toml_edit::Array>());
        }
        match settings.as_inline_table().filter(|o| !o.is_empty()) {
            Some(o) => {
                let mut st = Table::new();
                for (k, v) in o.iter() {
                    st[k] = Item::Value(v.clone());
                }
                t.insert("settings", Item::Table(st));
            }
            None => {
                t.remove("settings");
            }
        }
        Ok(())
    }

    pub fn set_preset_enabled(&mut self, cfg: &Config, name: &str, on: bool) -> Result<()> {
        let p = crate::preset::get(cfg, name)
            .ok_or_else(|| crate::error::UserError::not_found(crate::preset::unknown(cfg, name)))?;
        let t = self.table_at(&["preset", name])?;
        if p.builtin && !t.contains_key("profiles") {
            t["description"] = value(p.description.as_str());
            t["profiles"] = value(p.profiles.iter().collect::<toml_edit::Array>());
            if let Some(o) = p.extra.as_object().filter(|o| !o.is_empty()) {
                let mut st = Table::new();
                for (k, v) in o {
                    st[k.as_str()] = Item::Value(json_to_toml(v));
                }
                t.insert("settings", Item::Table(st));
            }
        }
        if on {
            t.remove("enabled");
        } else {
            t["enabled"] = value(false);
        }
        Ok(())
    }

    fn group_table(&mut self, name: &str) -> Option<&mut Table> {
        self.tables("group")
            .iter_mut()
            .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
    }

    pub fn set_group(&mut self, name: &str, key: &str, v: Option<toml_edit::Value>) -> Result<()> {
        let t = self.group_table_checked(name)?;
        match v {
            Some(v) => {
                t[key] = Item::Value(v);
            }
            None => {
                t.remove(key);
            }
        }
        Ok(())
    }

    pub fn add_group(&mut self, g: &crate::config::Group) -> Result<()> {
        if self.group_table(&g.name).is_some() {
            bail!("group '{}' already exists", g.name);
        }
        let mut t = Table::new();
        t["name"] = value(g.name.as_str());
        t["paths"] = value(g.paths.iter().collect::<toml_edit::Array>());
        t["preset"] = value(g.preset.as_str());
        if let Some(a) = g.apply {
            t["apply"] = value(a.as_str());
        }
        if let Some(s) = &g.sources {
            t["sources"] = value(s.iter().collect::<toml_edit::Array>());
        }
        self.tables("group").push(t);
        Ok(())
    }

    pub fn remove_group(&mut self, name: &str) -> bool {
        let groups = self.tables("group");
        let before = groups.len();
        groups.retain(|t| t.get("name").and_then(|v| v.as_str()) != Some(name));
        groups.len() != before
    }

    /// Give `path` to `owner` (a seat, "any" or "none"), replacing an entry for the same folder.
    pub fn link(&mut self, path: &Path, owner: &str) {
        let shown = paths::contract(path);
        let key = key_string(path);
        let folders = self.tables("folder");
        if let Some(t) = folders.iter_mut().find(|t| {
            t.get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|p| key_string(&paths::expand(p)) == key)
        }) {
            t["owner"] = value(owner);
            return;
        }
        let mut t = Table::new();
        t["path"] = value(shown);
        t["owner"] = value(owner);
        folders.push(t);
    }

    /// Remove the entry for exactly this folder. Returns whether there was one.
    pub fn unlink(&mut self, path: &Path) -> bool {
        let key = key_string(path);
        let folders = self.tables("folder");
        let before = folders.len();
        folders.retain(|t| {
            !t.get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|p| key_string(&paths::expand(p)) == key)
        });
        folders.len() != before
    }

    /// Remove every folder entry whose folder doesn't exist. Returns the removed paths, as written in the config.
    pub fn unlink_missing(&mut self) -> Vec<String> {
        let folders = self.tables("folder");
        let mut removed = Vec::new();
        folders.retain(|t| {
            let Some(p) = t.get("path").and_then(|v| v.as_str()) else {
                return true;
            };
            if paths::expand(p).exists() {
                true
            } else {
                removed.push(p.to_string());
                false
            }
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_keep_comments_and_validate() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        std::fs::write(
            &path,
            "# my notes\nenforcement = \"warn\"\n\n[[seat]]\nname = \"work\"\nconfig_dir = \"~/.cw\"\n",
        )
        .unwrap();
        let mut f = ConfigFile::open_at(&path).unwrap();
        f.add_seat("home", "~/.ch", &["h".into()]).unwrap();
        assert!(f.add_seat("h", "~/.x", &[]).is_err(), "alias taken");
        f.pin(
            "h",
            Some("org-1"),
            Some("Org"),
            Some("me@x.org"),
            Some("pro"),
        )
        .unwrap();
        let proj = paths::home().join("some-project");
        f.link(&proj, "home");
        f.link(&proj, "work"); // same folder: replaced, not added
        f.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my notes"), "{text}");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.folders.len(), 1);
        assert_eq!(cfg.folders[0].owner, "work");
        assert_eq!(cfg.folders[0].path, "~/some-project");
        assert_eq!(cfg.seat("home").unwrap().email.as_deref(), Some("me@x.org"));

        let mut f = ConfigFile::open_at(&path).unwrap();
        assert!(f.unlink(&proj));
        assert!(!f.unlink(&proj));
        f.link(&proj, "ghost");
        assert!(f.save().is_err(), "unknown seat is refused before writing");
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("some-project")
        );
    }
}
