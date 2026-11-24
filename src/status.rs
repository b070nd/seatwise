//! The state of each seat: who it is signed in as, whether that matches its pin, and whether its hooks are in place.
//! Used by `seat list` and `seat doctor`.

use crate::claude::{self, Account, HookState};
use crate::config::{Config, Enforcement, Link, Seat};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    SignedOut,
    /// The account is still recorded, but the seat's login (credentials) is gone: signed out in practice.
    LoggedOut,
    /// Signed in, but the seat has no org or email pinned: nothing to check against (`seat pin`).
    Unpinned,
    Ok,
    Wrong(String),
}

pub struct SeatStatus<'a> {
    pub seat: &'a Seat,
    pub account: Option<Account>,
    pub identity: Identity,
    pub enforcement: Enforcement,
    pub hooks: HookState,
}

impl<'a> SeatStatus<'a> {
    pub fn of(cfg: &Config, seat: &'a Seat, exe: &Path) -> Self {
        let account = claude::read_account(seat);
        // A Claude Code seat can keep its account in `.claude.json` after its login is gone.
        let logged_out = account.is_some()
            && seat.agent == crate::config::Agent::Claude
            && !crate::usage::has_login(seat);
        let identity = match &account {
            Some(_) if logged_out => Identity::LoggedOut,
            None => Identity::SignedOut,
            Some(_) if seat.org_uuid.is_none() && seat.email.is_none() => Identity::Unpinned,
            Some(a) => {
                match crate::guard::check_identity(seat, a.org_uuid.as_deref(), a.email.as_deref())
                {
                    Ok(()) => Identity::Ok,
                    Err(m) => Identity::Wrong(m.to_string()),
                }
            }
        };
        SeatStatus {
            seat,
            identity,
            enforcement: cfg.enforcement_of(seat),
            hooks: claude::hook_state(seat, exe),
            account,
        }
    }

    /// Hooks are what the enforcement level calls for.
    pub fn hooks_ok(&self) -> bool {
        match self.enforcement {
            Enforcement::Off => self.hooks == HookState::Missing,
            _ => self.hooks == HookState::Current,
        }
    }

    pub fn account_label(&self) -> String {
        if self.identity == Identity::LoggedOut {
            return "signed out (login gone)".into();
        }
        match &self.account {
            None => "signed out".into(),
            Some(a)
                if a.email.is_none() && a.org_type.as_deref() == Some(crate::claude::API_KEY) =>
            {
                "API key".into()
            }
            Some(a) => {
                let who = a.email.as_deref().unwrap_or("?");
                match a.org_name.as_deref() {
                    Some(org) => format!("{who} ({org})"),
                    None => who.to_string(),
                }
            }
        }
    }

    /// `Team premium`, `Pro +extra` (extra usage billed), `-` when signed out.
    pub fn plan_label(&self) -> String {
        if self.identity == Identity::LoggedOut {
            return "-".into();
        }
        let Some(a) = &self.account else {
            return "-".into();
        };
        let plan = crate::plan::Plan::of(a).map_or_else(|| "?".into(), |p| p.label());
        if a.extra_usage {
            format!("{plan} +extra")
        } else {
            plan
        }
    }

    pub fn identity_label(&self) -> &'static str {
        match self.identity {
            Identity::SignedOut => "-",
            Identity::LoggedOut => "SIGNED OUT",
            Identity::Unpinned => "not pinned",
            Identity::Ok => "ok",
            Identity::Wrong(_) => "WRONG",
        }
    }

    pub fn hooks_label(&self) -> &'static str {
        match (self.enforcement, self.hooks) {
            (Enforcement::Off, HookState::Missing) => "-",
            (Enforcement::Off, _) => "left over",
            (_, HookState::Current) => "ok",
            (_, HookState::Missing) => "MISSING",
            (_, HookState::Stale) => "stale",
        }
    }

    pub fn link_label(&self) -> String {
        match self.seat.link() {
            Link::System => "system".into(),
            Link::Copy => "copy".into(),
            Link::Command(argv) => argv.first().cloned().unwrap_or_default(),
        }
    }
}

/// Plain-text table, columns padded to their widest cell.
pub fn table(rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..rows.first().map_or(0, Vec::len))
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        out += line.join("  ").trim_end();
        out.push('\n');
    }
    out
}

pub fn list(cfg: &Config, exe: &Path) -> String {
    let mut rows = vec![
        [
            "SEAT", "CONFIG", "ACCOUNT", "PLAN", "IDENTITY", "ENFORCE", "HOOKS", "LINKS",
            "PROFILE", "FOLDERS",
        ]
        .map(String::from)
        .to_vec(),
    ];
    for seat in &cfg.seats {
        let st = SeatStatus::of(cfg, seat, exe);
        let mut name = if seat.aliases.is_empty() {
            seat.name.clone()
        } else {
            format!("{} ({})", seat.name, seat.aliases.join(", "))
        };
        if seat.agent != crate::config::Agent::Claude {
            name += &format!(" [{}]", seat.agent.as_str());
        }
        if !cfg.is_enabled(seat) {
            name += " (off)";
        }
        rows.push(vec![
            name,
            crate::paths::contract(&seat.config_dir()),
            st.account_label(),
            st.plan_label(),
            st.identity_label().into(),
            st.enforcement.as_str().into(),
            st.hooks_label().into(),
            st.link_label(),
            seat.profile.clone().unwrap_or_else(|| "-".into()),
            cfg.folders_of(&seat.name).len().to_string(),
        ]);
    }
    let mut out = table(&rows);
    let open: Vec<&str> = cfg
        .seats
        .iter()
        .filter(|s| cfg.enforcement_of(s) == crate::config::Enforcement::Off)
        .map(|s| s.name.as_str())
        .collect();
    if !open.is_empty() {
        out += &format!(
            "\nNot isolated (enforcement off): {}. Their sessions can read every seat's folders; \
             set enforcement = \"warn\" or \"strict\".\n",
            open.join(", ")
        );
    }
    out
}
