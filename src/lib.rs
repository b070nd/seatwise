//! seatwise: keep several Claude Code subscriptions ("seats") out of each other's work, each with its own
//! config folder, organization and project folders, and optional hooks that guard the folders.

pub mod claude;
pub mod config;
pub mod desktop;
pub mod doctor;
pub mod edit;
pub mod folders;
pub mod guard;
pub mod home;
pub mod hybrid;
pub mod init;
pub mod launch;
pub mod link;
pub mod paths;
pub mod plan;
pub mod preset;
pub mod privacy;
pub mod ps;
pub mod sentinel;
pub mod share;
pub mod shell;
pub mod shim;
pub mod skills;
pub mod status;
pub mod sync;
pub mod term;
pub mod tui;
pub mod usage;
