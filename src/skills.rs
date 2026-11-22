//! The seatwise skillset: skills that let a session (in the desktop app or the CLI) open a terminal for the
//! project, report the seat's status and usage, and work from the seat's home repo. `seat skills install` puts
//! them where each seat's agent finds them, with the path of this `seat` binary filled in.

use crate::config::{Agent, Seat};
use crate::paths;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const SKILLS: [(&str, &str); 3] = [
    (
        "seat-terminal",
        include_str!("../skills/seat-terminal/SKILL.md"),
    ),
    (
        "seat-status",
        include_str!("../skills/seat-status/SKILL.md"),
    ),
    ("seat-home", include_str!("../skills/seat-home/SKILL.md")),
];

/// Where a seat's agent looks for user skills. Codex has no per-account skills folder: its skills go in the
/// seat's home repo (`.agents/skills`), or else in `~/.agents/skills`, which every Codex seat shares.
pub fn target(seat: &Seat) -> PathBuf {
    match seat.agent {
        Agent::Claude => seat.config_dir().join("skills"),
        Agent::Codex => match seat.home() {
            Some(h) => h.join(".agents").join("skills"),
            None => paths::home().join(".agents").join("skills"),
        },
    }
}

pub fn render(text: &str, exe: &Path) -> String {
    text.replace("{{seat}}", &exe.display().to_string())
}

/// Write the skills for a seat. Returns the folder they went to.
pub fn install(seat: &Seat, exe: &Path) -> Result<PathBuf> {
    let dir = target(seat);
    for (name, text) in SKILLS {
        let file = dir.join(name).join("SKILL.md");
        std::fs::create_dir_all(file.parent().expect("has parent"))?;
        std::fs::write(&file, render(text, exe))
            .with_context(|| format!("writing {}", file.display()))?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_have_frontmatter_and_the_binary_path() {
        for (name, text) in SKILLS {
            assert!(
                text.starts_with(&format!("---\nname: {name}\ndescription: ")),
                "{name}"
            );
            let out = render(text, Path::new("/x/seat"));
            assert!(
                out.contains("\"/x/seat\"") && !out.contains("{{seat}}"),
                "{name}"
            );
        }
    }
}
