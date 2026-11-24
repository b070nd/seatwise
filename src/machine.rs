//! The machine-readable interface: `--json` on `owner`, `list`, `ps`, `explain` and `doctor`, for orchestrators (bb,
//! IDEs, CI, agent frameworks) that need to ask "which seat?" before they start an agent.
//!
//! Every document is a JSON object with `"schema": SCHEMA`. Within a schema version fields are only ever added;
//! renaming or removing one, or changing its meaning, raises the version. Account identifiers (org UUIDs and names,
//! emails, account ids) are never part of it, and the usual redaction applies on top.

use crate::claude::{self, HookState};
use crate::config::{Config, Enforcement, Owner, Seat};
use crate::status::{Identity, SeatStatus};
use crate::{doctor, folders, paths, preset, ps};
use serde_json::{Value, json};
use std::path::Path;

/// The version of the JSON documents below.
pub const SCHEMA: u32 = 1;

fn hooks(state: HookState) -> &'static str {
    match state {
        HookState::Missing => "missing",
        HookState::Current => "current",
        HookState::Stale => "outdated",
    }
}

/// `seat owner --json`: who owns a folder and which seat `seat run auto` would start there.
pub fn owner(cfg: &Config, path: &Path) -> Value {
    let (kind, owner) = match folders::owner_of(cfg, path) {
        Some(Owner::Seat(s)) => ("seat", Some(s.to_string())),
        Some(Owner::Any) => ("any", None),
        Some(Owner::Nobody) => ("none", None),
        None => ("unassigned", None),
    };
    let run = crate::launch::auto_seat(cfg, path);
    json!({
        "schema": SCHEMA,
        "path": path.display().to_string(),
        "kind": kind,
        "owner": owner,
        "run_seat": run.map(|s| s.name.clone()),
        "run_agent": run.map(|s| s.agent.as_str()),
        "enforcement": run.map(|s| cfg.enforcement_of(s).as_str()),
    })
}

/// `seat list --json`: every seat's state.
pub fn seats(cfg: &Config, exe: &Path) -> Value {
    let seats: Vec<Value> = cfg
        .seats
        .iter()
        .map(|seat| seat_doc(cfg, seat, exe))
        .collect();
    json!({ "schema": SCHEMA, "seats": seats })
}

fn seat_doc(cfg: &Config, seat: &Seat, exe: &Path) -> Value {
    let st = SeatStatus::of(cfg, seat, exe);
    let identity = match st.identity {
        Identity::SignedOut => "signed_out",
        Identity::LoggedOut => "logged_out",
        Identity::Unpinned => "unpinned",
        Identity::Ok => "ok",
        Identity::Wrong(_) => "wrong",
    };
    let folders: Vec<String> = cfg
        .folders_of(&seat.name)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    json!({
        "name": seat.name,
        "aliases": seat.aliases,
        "agent": seat.agent.as_str(),
        "enabled": cfg.is_enabled(seat),
        "config_dir": seat.config_dir().display().to_string(),
        "signed_in": st.account.is_some() && st.identity != Identity::LoggedOut,
        "identity": identity,
        "pinned": seat.org_uuid.is_some() || seat.email.is_some(),
        "plan": st.plan_label(),
        "enforcement": st.enforcement.as_str(),
        "isolated": st.enforcement != Enforcement::Off && st.hooks_ok(),
        "hooks": hooks(st.hooks),
        "hook_contract": claude::hook_contract(seat),
        "link": st.link_label(),
        "profile": seat.profile,
        "folders": folders,
    })
}

/// `seat ps --json`: the agent sessions running now.
pub fn sessions(cfg: &Config) -> anyhow::Result<Value> {
    let list: Vec<Value> = ps::sessions(cfg)?
        .into_iter()
        .map(|s| {
            json!({
                "seat": s.seat,
                "pid": s.pid,
                "minutes": s.minutes,
                "preset": s.preset,
                "cwd": s.cwd.map(|c| c.display().to_string()),
            })
        })
        .collect();
    Ok(json!({ "schema": SCHEMA, "sessions": list }))
}

/// `seat explain --json`: what a session in a folder would run with.
pub fn explain(cfg: &Config, seat: &Seat, dir: &Path, with: &[String]) -> Value {
    let mut doc = preset::explain_doc(cfg, seat, dir, with);
    doc["schema"] = json!(SCHEMA);
    doc
}

/// `seat doctor --json`: the findings; `ok` is false when one is an error (the exit code is 1 then, too).
pub fn doctor(findings: &[doctor::Finding]) -> Value {
    let list: Vec<Value> = findings
        .iter()
        .map(|f| {
            json!({
                "level": match f.level {
                    doctor::Level::Error => "error",
                    doctor::Level::Warn => "warn",
                    doctor::Level::Info => "info",
                },
                "message": f.message,
                "fixable": f.fix.is_some(),
            })
        })
        .collect();
    json!({
        "schema": SCHEMA,
        "ok": !findings.iter().any(|f| f.level == doctor::Level::Error),
        "config": paths::config_file().display().to_string(),
        "config_format": crate::config::CONFIG_VERSION,
        "hook_contract": claude::HOOK_CONTRACT,
        "findings": list,
    })
}
