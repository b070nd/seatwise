//! `seat preset new` / `seat preset edit`: a step-by-step wizard over a [`Draft`]. It only offers what the draft
//! allows, so whatever it writes is valid. Reads answers line by line, so it works in any terminal and in tests.

use crate::catalog::{self, Level};
use crate::config::Config;
use crate::draft::Draft;
use anyhow::Result;
use std::io::{BufRead, Write};

const MENU: &str = "[a]dd or change a setting  [x] experimental  [p]rofiles  [r]emove  [d]escription  [v]iew result  [w]rite  [q]uit";

fn ask(input: &mut impl BufRead, out: &mut impl Write, prompt: &str) -> Result<Option<String>> {
    write!(out, "{prompt}")?;
    out.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

fn problems(out: &mut impl Write, d: &Draft, cfg: &Config) -> Result<()> {
    let p = d.problems(cfg);
    if !p.is_empty() {
        write!(out, "{}", catalog::report(&p))?;
    }
    Ok(())
}

/// Pick a setting from the options matching a search, then a value for it.
fn add(
    input: &mut impl BufRead,
    out: &mut impl Write,
    cfg: &Config,
    d: &mut Draft,
    experimental: bool,
) -> Result<()> {
    let query = if experimental {
        String::new()
    } else {
        match ask(input, out, "search settings (words, empty for all): ")? {
            Some(q) => q,
            None => return Ok(()),
        }
    };
    let options = d.options(cfg, &query, experimental);
    if options.is_empty() {
        writeln!(out, "nothing matches that can go in this preset")?;
        return Ok(());
    }
    let shown: Vec<_> = options.iter().take(25).collect();
    for (i, o) in shown.iter().enumerate() {
        let now = o
            .current
            .as_ref()
            .map(|v| format!(" [now {v}]"))
            .unwrap_or_default();
        let exp = if o.entry.experimental() {
            " (experimental)"
        } else {
            ""
        };
        let blocked = o
            .blocked
            .as_ref()
            .map(|b| format!("\n       not now: {b}"))
            .unwrap_or_default();
        writeln!(
            out,
            "  {:>2}) {}{exp}{now}\n       {}{blocked}",
            i + 1,
            o.entry.name,
            o.entry.meaning
        )?;
    }
    if options.len() > shown.len() {
        writeln!(
            out,
            "  … {} more: narrow the search",
            options.len() - shown.len()
        )?;
    }
    let Some(pick) = ask(input, out, "number (empty to go back): ")? else {
        return Ok(());
    };
    let Some(choice) = pick
        .parse::<usize>()
        .ok()
        .and_then(|n| shown.get(n.wrapping_sub(1)))
    else {
        return Ok(());
    };
    let e = choice.entry;
    if let Some(n) = &e.note {
        writeln!(out, "  {n}")?;
    }
    let raw = if choice.values.is_empty() {
        match ask(
            input,
            out,
            &format!("value for {} ({}): ", e.name, e.type_label()),
        )? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(()),
        }
    } else {
        for (i, v) in choice.values.iter().enumerate() {
            writeln!(out, "  {}) {v}", i + 1)?;
        }
        match ask(input, out, "value (number or text): ")? {
            Some(v) => v
                .parse::<usize>()
                .ok()
                .and_then(|n| choice.values.get(n.wrapping_sub(1)).cloned())
                .unwrap_or(v),
            None => return Ok(()),
        }
    };
    match d.set(cfg, &e.name, &raw) {
        Ok(notes) => {
            writeln!(out, "  set {} = {raw}", e.name)?;
            for n in notes {
                writeln!(out, "  note: {}", n.message)?;
            }
        }
        Err(err) => writeln!(out, "  {err}")?,
    }
    Ok(())
}

/// Run the wizard. Returns the draft to write, or `None` when the person quits without writing.
pub fn run(
    cfg: &Config,
    mut d: Draft,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<Option<Draft>> {
    let start = d.clone();
    writeln!(
        out,
        "Building preset '{}'. Only settings Claude Code honors in {} are offered; choices that clash are refused.",
        d.name,
        d.target.label()
    )?;
    loop {
        writeln!(out, "\n{}", d.summary().trim_end())?;
        problems(out, &d, cfg)?;
        writeln!(out, "{MENU}")?;
        let Some(cmd) = ask(input, out, "> ")? else {
            return Ok(None);
        };
        match cmd.as_str() {
            "a" => add(input, out, cfg, &mut d, false)?,
            "x" => add(input, out, cfg, &mut d, true)?,
            "p" => {
                let names: Vec<String> =
                    crate::plan::all(cfg).into_iter().map(|(n, _)| n).collect();
                writeln!(out, "  profiles: {}", names.join(", "))?;
                if let Some(line) = ask(
                    input,
                    out,
                    "profiles in order (space-separated, empty for none): ",
                )? {
                    let list: Vec<String> = line.split_whitespace().map(str::to_string).collect();
                    if let Err(e) = d.set_profiles(cfg, &list) {
                        writeln!(out, "  {e}")?;
                    }
                }
            }
            "r" => {
                if let Some(key) = ask(input, out, "setting to remove: ")?
                    && !d.unset(&key)
                {
                    writeln!(out, "  {key} isn't set in this preset")?;
                }
            }
            "d" => {
                if let Some(text) = ask(input, out, "description: ")? {
                    d.description = text;
                }
            }
            "v" => writeln!(out, "{}", serde_json::to_string_pretty(&d.preview(cfg))?)?,
            "w" => {
                if d.problems(cfg).iter().any(|p| p.level != Level::Info) {
                    writeln!(out, "  fix the problems above first")?;
                    continue;
                }
                return Ok(Some(d));
            }
            "q" => {
                if d != start
                    && ask(input, out, "quit without writing? [y/N] ")?.is_none_or(|a| a != "y")
                {
                    continue;
                }
                return Ok(None);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_adds_an_experimental_setting_and_writes() {
        let cfg: Config = toml::from_str("").unwrap();
        // x → pick the first experimental option offered … then the value, then write.
        let d = Draft::new("fast");
        let opts = d.options(&cfg, "", true);
        let n = opts
            .iter()
            .position(|o| o.entry.name == "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE")
            .unwrap()
            + 1;
        let script = format!("d\nquick with code mode\nx\n{n}\n1\na\neffort\n");
        let script = script
            + &{
                let o = d.options(&cfg, "effort", false);
                let i = o
                    .iter()
                    .position(|o| o.entry.name == "effortLevel")
                    .unwrap()
                    + 1;
                format!("{i}\nhigh\nw\n")
            };
        let mut out = Vec::new();
        let got = run(&cfg, d, &mut script.as_bytes(), &mut out)
            .unwrap()
            .expect("written");
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            got.settings["env"]["CLAUDE_CODE_EXPERIMENTAL_CODE_MODE"], "1",
            "{text}"
        );
        assert_eq!(got.settings["effortLevel"], "high", "{text}");
        assert_eq!(got.description, "quick with code mode");
        assert!(text.contains("(experimental)"), "{text}");
    }

    #[test]
    fn quitting_with_changes_asks_first() {
        let cfg: Config = toml::from_str("").unwrap();
        let mut out = Vec::new();
        let got = run(
            &cfg,
            Draft::new("x"),
            &mut "d\nchanged\nq\nn\nq\ny\n".as_bytes(),
            &mut out,
        )
        .unwrap();
        assert!(got.is_none());
    }
}
