# seatwise

**Agentic development seats manager: keep your Claude Code and Codex accounts from crossing into each other's work.**

[![ci](https://github.com/b070nd/seatwise/actions/workflows/ci.yml/badge.svg)](https://github.com/b070nd/seatwise/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/seatwise.svg)](https://crates.io/crates/seatwise)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

If you have a company seat and a personal plan (or two clients' seats), seatwise gives each one its own config folder,
login and organization, ties your project folders to the seat they belong to, and can stop a session on one seat from
reading, editing or prompting in another seat's folders.

> Status: early (0.x). Works on macOS, Linux and Windows. Not affiliated with Anthropic or OpenAI.

```text
$ seat list
SEAT          CONFIG              ACCOUNT                         PLAN          IDENTITY  ENFORCE  HOOKS  LINKS    PROFILE   FOLDERS
work (w)      ~/.claude-work      <email:work> (<org-name:work>)  Team premium  ok        warn     ok     firefox  balanced  3
personal (p)  ~/.claude-personal  <email:personal>                Max 5x        ok        warn     ok     copy     heavy     2
codex         ~/.codex            API key                         pay per use   -         off      -      system   -         1
```

*Illustrative output, shown the way an agent or a pipe sees it: account identifiers are replaced by placeholders.*

## Why

Switching accounts is easy: one `CLAUDE_CONFIG_DIR` per account, and plenty of tools wrap it. What switching alone doesn't
cover:

- **A session wanders into the wrong folder.** A company session can `Read`, `Grep` or `cat` your personal projects (or a
  client's) as easily as its own. Nothing tells you, and the content goes to that seat's organization.
- **`/login` lands on the wrong organization.** With the same email in two orgs, it's easy to pick the wrong one, and you only
  notice later.
- **Links open in the wrong browser.** A login or OAuth link from the company seat opens in your personal browser profile.

seatwise handles these. If you only need to switch accounts, a plain alias
(`alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'`) or one of the many account switchers will do.

## What it does

- **Isolation between seats** (`enforcement = "warn"` or `"strict"`): two Claude Code hooks check every prompt and every file
  and shell tool call. In `warn`, you're told and asked before a tool touches another seat's folder; in `strict`, it's
  blocked. Prompts on the wrong organization are flagged or blocked the same way.
- **Folders pick the seat:** each folder belongs to a seat, is shared, or is closed to all. `seat run auto` (or plain `claude`,
  if you turn it on) starts the seat that owns the current folder.
- **One command per seat:** `claude-<seat>` starts Claude Code with that seat's `CLAUDE_CONFIG_DIR`, and without a stray
  `ANTHROPIC_API_KEY`.
- **Wrong-org warning at launch,** whatever the enforcement level.
- **Links go where you want, per seat:** the default browser, a specific browser or browser profile (any command), or the
  clipboard so you can paste them yourself. This includes `/login`.
- **Shared start folder:** start any seat from one neutral folder, with that seat's own folders available.
- **Desktop app and terminal, side by side:** `seat term` opens a terminal on the same seat, in the same project,
  optionally with session-only settings (`--with experimental`). Installed as a skill (`seat skills install`), the
  desktop app can do it for you when you ask for "a terminal for this project".
- **A home repo per seat** ([the home-repo pattern](https://www.aihero.dev/the-home-repo-pattern)): one repo with the
  seat's skills, `AGENTS.md` and docs, where sessions start, and a generated list of pointers to that seat's repos only.
- **Codex seats (ChatGPT or API key):** `agent = "codex"` gives a seat its own `CODEX_HOME`, with the same folders, hooks and links.
- **Use-case presets for folder groups:** `explore`, `build`, `review`, `quick` or your own, given to folders the
  way seats are (longest path wins), applied per session or written into the project. Same folder, same settings,
  every time; `seat explain` shows every layer and what it set.
- **A sentinel** that checks, on start or on a schedule, for settings that cost more than they need to, and only
  reports.
- **`seat tui`:** turn seats, agents, presets, groups and the sentinel on and off in one screen.
- **Optional settings snapshots:** tracked files (e.g. `settings.json`) are checked on every start; changes are kept in
  history and `seat sync restore <seat>` brings the old version back.

Enforcement is off until you choose a level, so seatwise never blocks anything you didn't ask it to.

## Install

Any of these gives you the `seat` binary:

- **From crates.io** (released versions):

  ```sh
  cargo install seatwise --locked
  ```

- **From GitHub** (the latest `main`, or a tag with `--tag v0.1.0`):

  ```sh
  cargo install --git https://github.com/b070nd/seatwise --locked
  ```

- **Prebuilt binaries** for Linux, macOS and Windows (x86_64 and arm64) are on the
  [releases page](https://github.com/b070nd/seatwise/releases), with `checksums.txt` and build provenance
  (`gh attestation verify <archive> -R b070nd/seatwise`). Put `seat` somewhere on your `PATH`.

Update the same way you installed: `cargo install seatwise --locked` again, or download the newer release.

## Quick start

```sh
seat init                     # finds your Claude config folders, pins each one's account, proposes folders
seat doctor                   # what to fix; `seat doctor --fix` installs or removes hooks
echo 'eval "$(seat shell-init zsh)"' >> ~/.zshrc        # bash | zsh | fish | powershell
```

PowerShell: add `seat shell-init powershell | Out-String | Invoke-Expression` to `$PROFILE`.

Then turn on isolation: set `enforcement = "warn"` (or `"strict"`) in the config and run `seat install-hooks`.

## Commands

| | |
|---|---|
| `seat init` | Propose a config from the config folders on this machine: seats, pinned accounts, folders from session history |
| `seat add <name> [--from <seat>]` | New seat (or adopt a folder with `--config-dir`); `--from` copies another seat's setup, never its login |
| `seat login <seat>` / `seat pin <seat>` | Sign a seat in; record its org and email so a wrong login is caught |
| `seat link <seat\|any\|none> [path]`, `seat unlink`, `seat owner` | Give folders to seats |
| `claude-<seat>`, `seat run <seat\|auto>` | Start Claude Code on a seat |
| `seat list` | Every seat: account, identity check, enforcement, hooks, link mode, folders |
| `seat ps` | Claude Code sessions running now, by seat |
| `seat profile <seat> [name] [--usage]` | The seat's plan, and the usage profile that fits it (`light`, `balanced`, `heavy`, or your own); with a name, apply it. `seat login` offers it right after signing in |
| `seat usage [seat]` | 5-hour and weekly limits used, per seat (see below) |
| `seat doctor [--fix]` | Missing or stale hooks, wrong or unpinned accounts, two seats on one account, `CLAUDE.md` leaking from a parent folder, stray API keys |
| `seat share [seat]` | Copy shared skills, agents, commands into seats (also on every start) |
| `seat shim install` | A `claude` on PATH that picks the seat by folder, for IDE terminals and tools that start `claude` themselves |
| `seat run --with <profile> <seat>` | Start with a usage profile for this session only (passed as `--settings`) |
| `seat term [--seat s] [--with p] [--dir d] [--print]` | New terminal window running the seat in a project; from inside a session it takes that session's seat and project |
| `seat hybrid [--check] [--link] [--preset p] [--with p]` | Set up the command line for this project next to the desktop app, then open a terminal there |
| `seat skills install [seat]` / `seat skills show <name>` | Install the seatwise skills (`seat-hybrid`, `seat-terminal`, `seat-status`, `seat-home`) into seats |
| `seat home init <seat> [path]` / `seat home sync` | Create or adopt a seat's home repo; refresh its pointers. `seat run --home <seat>` starts there |
| `seat tui` | One screen for seats, agents, presets, folder groups and general options; `w` writes, keeping comments |
| `seat enable/disable <target>` | A seat, `agent:claude\|codex`, `preset:<name>`, `group:<name>` or `sentinel` |
| `seat preset list` / `seat preset show <name>` | Use-case presets and their resolved settings |
| `seat group add [folder]` / `list` / `remove` | Give folders a preset; asks for preset and apply mode on a terminal |
| `seat explain [folder]` | What a session there runs with: group, preset, each settings layer, and which layer set each value |
| `seat sentinel [--here]` / `install` / `rules` | Find settings worth changing; `install` schedules it every `sentinel.interval` |
| `seat completions <shell>` | Shell completions |

## Plans and usage

`seat list` shows each seat's plan (Pro, Max 5x/20x, Team standard/premium, Enterprise) and `+extra` when usage beyond
the plan is billed. `seat pin` records the plan, and `seat doctor` reports when it changes.

A usage profile is a few settings merged into the seat's `settings.json` (never its hooks): main model, subagent model,
effort and extended thinking. `seat profile <seat>` suggests the one that fits the plan, one step lighter when this
week's limit is mostly used (`--usage`). Define your own as `[profile.<name>]` tables in the config.

`seat usage` asks the endpoint Claude Code's `/usage` uses, with the seat's own login (its `.credentials.json`, or on
macOS its keychain item, which may ask you to allow access). That endpoint is undocumented and may change. The token
is sent only there, never shown or stored, and never refreshed: an expired one means starting the seat once.

## Presets and folder groups

Claude Code builds a session's settings from several layers: your `settings.json`, the project's `.claude/settings.json`
and `.claude/settings.local.json`, and command-line flags. It also writes to them itself (`/model`, "always allow"
answers), so the same project can run differently from one week to the next.

seatwise makes the part it controls the same every time. A **preset** is a use case: usage profiles combined with a
few settings (model, effort, thinking, permission mode). A **folder group** gives a preset to folders, and the longest
matching path wins, as with seats. A nested group with `preset = "none"` opts a project out; a disabled group falls back
to its parent.

| Preset | Profile | Permission mode | For |
|---|---|---|---|
| `explore` | light | plan | reading and planning |
| `build` | balanced | acceptEdits | day-to-day coding |
| `review` | heavy | plan | careful review |
| `quick` | light | (unchanged) | small fixes |

Each group chooses how its preset arrives (`apply`, default `preset_apply`):
- `session`: passed as `--settings` when seatwise starts the session. Nothing on disk changes, and `--settings` outranks
  the project's files.
- `project`: written into `.claude/settings.local.json` on start, so a plain `claude` gets it too.

`sources = ["user", "local"]` loads only those layers (Claude Code's `--setting-sources`), so a project's committed
`settings.json` can't change how your sessions run. `seat explain` prints the layers in order and, for every value,
the layer that set it. Each start gets a fingerprint of what seatwise added (`review@3f9a1c02`); `seat ps` shows it, so
two sessions with the same fingerprint run with the same settings from seatwise.

## Sentinel

`seat sentinel` checks every enabled seat and its projects against a fixed list of rules (`seat sentinel rules`):

| Rule | Finds |
|---|---|
| `plan-fit` | Opus as the main model, in a seat's settings or a group's preset, on Pro or Team standard |
| `instructions-size` | CLAUDE.md files over 40,000 characters, which are loaded into every session |
| `mcp-unused` | project MCP servers that none of the last 20 sessions used |
| `allow-list` | permission allow lists that repeat rules or have piled up past 40 entries |
| `preset-drift` | project settings that differ from the group's preset |

Each finding names its rule, its scope (a seat or one project), the evidence and the change to make. The sentinel never
makes the change: that keeps your settings yours, and the results the same for the same files. Silence a rule with
`sentinel.ignore` (`"rule"`, or `"rule@<folder>"` for one project).

With `sentinel.enabled`, a seat's start checks that seat and project at most once per `sentinel.interval`. `seat
sentinel install` also runs it on that interval in the background (a launchd agent, a systemd user timer, or a Task
Scheduler task) and shows findings as notifications; `--dry-run` prints what it would write.

## Desktop app, terminal and home repos

In the desktop app, chat as usual; when a task needs the CLI (a long-running command, an experimental feature, your own
shell), ask for a terminal. The `seat-terminal` skill runs `seat term`, which opens one on the same seat and project:

```sh
seat skills install            # every seat: seat-hybrid, seat-terminal, seat-status, seat-home
seat hybrid --check            # what is set up for this project, changing nothing
seat hybrid --link --preset build --with experimental
seat term --with experimental  # just the terminal, from any shell
```

`seat hybrid` sets up the command-line side in one step, then opens the terminal. It makes sure the project folder
belongs to the session's seat (`--link` assigns an unassigned one), installs the seat's skills, gives the project a
use-case preset (`--preset`), and opens a terminal with session-only settings (`--with`). It refuses a folder that
belongs to another seat. In the desktop app, ask for "hybrid mode" or "the CLI for this project"; the `seat-hybrid`
skill runs `--check` first and asks before assigning folders or presets.

`seat term` takes the seat from the session's config folder (or the org the desktop app exports, when exactly one seat
is pinned to it) and the project from `CLAUDE_PROJECT_DIR`. `terminal.profile` sets the profile it uses by default.

A home repo is where a seat's sessions start. `seat home init <seat>` creates `~/<seat>-home` with `AGENTS.md`,
`CLAUDE.md` (`@AGENTS.md`), `skills/` and `docs/`, and writes an "Other repos" block listing the seat's own git repos:
`- name (description): path. Clone from <remote> if missing.` The block is regenerated by `seat home sync`; the rest of
the file is yours. Skills in `skills/` are copied into the seat on every start, so they work in all its projects.
`seat doctor` warns when a home repo mentions another seat's folder.

## Codex (ChatGPT) seats

A seat with `agent = "codex"` runs the Codex CLI with its own `CODEX_HOME` (its `auth.json` and `config.toml`).
Folders, enforcement and link handling work the same; hooks go to `hooks.json`, and Codex asks you to approve them once
in `/hooks`. The tool hook scans every path in Codex's tool input, including `apply_patch` file headers. `seat list`
shows the ChatGPT plan, or "API key" for a seat signed in with an OpenAI API key (seatwise only checks that a key is
there; it never reads or keeps it). An API-key login has no account to pin, so the org and email checks don't apply to
it. Usage profiles and `seat usage` are Claude Code only. The exact Codex hook input is not yet checked against a real
Codex install, so treat Codex isolation as best effort for now.

## Privacy: identifiers stay on this computer

Org UUIDs, org names, emails, account ids and login tokens are read only from your own machine's files.
- When seatwise runs under an agent (Claude Code, Codex, or a session seatwise started) or its output isn't a
  terminal, every identifier is replaced by a placeholder naming its seat (`<org:work>`, `<email:work>`), and any
  other UUID or email address by `<uuid>` / `<email>`. Hook messages, which reach the model, never contain them.
- `SEATWISE_REVEAL=1` shows them when you pipe output in your own shell. It has no effect under an agent.
- Tokens are never printed. `seat usage` sends a seat's token only to Anthropic's usage endpoint, the same host
  Claude Code sends it to; seatwise refuses any other host.
- The skills tell agents not to read seatwise's config or any seat's account files.

## What seatwise leaves out, on purpose

- **Moving sessions between seats** and **switching accounts automatically when one runs out of quota:** both move
  work across the line seatwise exists to keep. `seat usage` shows the numbers; which seat to use stays your call.
- **A per-repo file naming the seat** (`.claude-profile` and the like): anyone can commit one into a repository, so under
  enforcement it would let a cloned repo choose its own seat. Folder ownership lives in your config only.
- **Changing `CLAUDE_CONFIG_DIR` on `cd`:** it leaks into every program started from that shell. Use `claude-<seat>`,
  `shell.override_claude`, or the shim.
- **Linking (symlinking) shared items:** copies work the same on every OS and can't be edited from one seat to change another.

## Limits (what is enforced and what isn't)

- Enforcement is off unless you turn it on. Without it, folders only decide which seat `seat run auto` picks.
- File tools (Read, Edit, Write, Glob, Grep, LS, NotebookEdit) are checked exactly, including nested folders and symlinks.
- Shell commands (Bash, PowerShell) are scanned for absolute and home-relative paths only; relative paths such as `../other`
  or paths built at runtime are not caught. Pair seatwise with Claude Code's sandbox if you need a hard boundary.
- The hooks guard Claude Code sessions. They don't stop you, or other programs, from opening the folders.
- For seats whose `link` is not `"system"`, `BROWSER` is set to the `seat` binary; keep it installed at a path without spaces.

## Platforms

| | macOS | Linux | Windows |
|---|---|---|---|
| Launcher, folder and org guard, hooks, settings snapshots | yes | yes | yes |
| Per-seat link handling (browser command, copy) | yes | yes (copy needs `wl-copy` or `xclip`) | yes |
| Desktop app seats and link router | planned | n/a (no desktop app) | planned |

## More

- [docs/architecture.md](docs/architecture.md): how the pieces fit, and the module map.
- [docs/security.md](docs/security.md): what seatwise protects, and what it does not.
- [CHANGELOG.md](CHANGELOG.md), [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution you submit for inclusion in this project, as defined in the
Apache-2.0 license, is dual licensed as above, without any additional terms or conditions.
