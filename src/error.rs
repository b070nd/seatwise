//! Errors the user can act on: what happened, what to do about it, and an exit code scripts can branch on.
//!
//! Commands return `anyhow::Result`; an error that is a [`UserError`] anywhere in its chain decides the exit code
//! and gets a `resolution:` line. Everything else is a plain failure (exit 1).
//!
//! | code | meaning |
//! |------|---------|
//! | 0 | success |
//! | 1 | failure (anything unexpected) |
//! | 2 | usage error (bad flag or argument; clap) |
//! | 3 | refused: the isolation guard stopped it (wrong folder, closed folder, wrong org) |
//! | 4 | not found: no such seat, preset or group |
//!
//! The hooks keep Claude Code's own convention: exit 2 blocks a prompt or tool call.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Failure = 1,
    Refused = 3,
    NotFound = 4,
}

#[derive(Debug)]
pub struct UserError {
    pub message: String,
    /// What the user can do about it, shown on its own line.
    pub resolution: Option<String>,
    pub code: Code,
}

impl UserError {
    pub fn new(message: impl Into<String>) -> Self {
        UserError {
            message: message.into(),
            resolution: None,
            code: Code::Failure,
        }
    }

    /// No such seat, preset or group.
    pub fn not_found(message: impl Into<String>) -> Self {
        UserError::new(message).code(Code::NotFound)
    }

    pub fn resolution(mut self, resolution: impl Into<String>) -> Self {
        self.resolution = Some(resolution.into());
        self
    }

    pub fn code(mut self, code: Code) -> Self {
        self.code = code;
        self
    }
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UserError {}

/// The lines to print for `error` and the exit code to return.
pub fn render(error: &anyhow::Error) -> (Vec<String>, u8) {
    let user = error.chain().find_map(|e| e.downcast_ref::<UserError>());
    let mut lines = vec![format!("seat: {error:#}")];
    if let Some(r) = user.and_then(|u| u.resolution.as_deref()) {
        lines.push(format!("  resolution: {r}"));
    }
    (lines, user.map_or(Code::Failure, |u| u.code) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn a_user_error_anywhere_in_the_chain_sets_the_code_and_resolution() {
        let e: anyhow::Error = UserError::new("no good")
            .resolution("do this")
            .code(Code::Refused)
            .into();
        let wrapped = Err::<(), _>(e).context("while starting").unwrap_err();
        let (lines, code) = render(&wrapped);
        assert_eq!(code, 3);
        assert_eq!(
            lines,
            ["seat: while starting: no good", "  resolution: do this"]
        );
        let (lines, code) = render(&anyhow::anyhow!("plain"));
        assert_eq!(
            (lines.as_slice(), code),
            (&["seat: plain".to_string()][..], 1)
        );
    }
}
