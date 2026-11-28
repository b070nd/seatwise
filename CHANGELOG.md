# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.2] - 2026-11-28

### Added
- Sessions seatwise starts keep the Mac awake while they run (`caffeinate -i`, idle sleep only), as the desktop app
  does for its own sessions, so a terminal session isn't cut off by sleep in the middle of a turn. On by default;
  `keep_awake = false` in the config turns it off.

### Fixed
- The terminal and hybrid skills open the terminal in the desktop app's Terminal panel, beside the conversation,
  instead of a new window: they name the app's terminal tool, which a session may list only as a deferred tool.

## [0.9.1] - 2026-11-26

### Changed
- The `terminal` and `hybrid` skills open the CLI in the desktop app's own Terminal panel, beside the conversation,
  when the session has a tool for it, instead of a separate terminal window. Elsewhere they open a window as before.

## [0.9.0] - 2026-11-26

### Added
- `seat tui` has a Running tab: per seat, its desktop window (on, off, or needing a restart) and its CLI sessions.
  `d` starts the seat's desktop app, `x` quits it, `R` restarts it on the seat (both ask first, since quitting stops
  its Code sessions, as Cmd-Q does), and `t` opens a terminal on the seat in its most recent project. `t` on the
  Projects tab opens one in that project.
- `seat desktop <seat> --quit` quits the seat's desktop window the way Cmd-Q does (the app runs its own quit, saving
  its state) and waits until it has gone; `--restart` then starts it on the seat. A Claude opened from the Dock on the
  seat's data folder counts as the seat's window, so `--restart` is the fix for it.

### Changed
- The hints for a Dock-opened or outdated desktop window name `seat desktop <seat> --quit` / `--restart` instead of
  quitting it by hand.

## [0.8.4] - 2026-11-25

### Removed
- `window` under `[seat.desktop]`, added in 0.8.3. It reset the window to one size on every start, so a size you
  resized to later was lost. The desktop app keeps a resized window's size itself; a window it restores maximized was
  saved while it covered a whole screen. A config that still sets `window` is refused; remove the line.

## [0.8.3] - 2026-11-25

### Added
- `window = "<width>x<height>"` under `[seat.desktop]`: every `seat desktop` start opens the seat's window at that
  size, never maximized or full screen. The app saves a window as maximized whenever it fills a screen's usable area
  and maximizes it on every start after that, so a resized window could keep coming back full screen.
- `seat desktop <seat> --own-folder` moves a seat's desktop data out of the app's usual folder into `Claude-<seat>`,
  login and sessions included (the app must be quit). While a seat uses the usual folder, every Claude opened from the
  Dock, Finder, Spotlight or a login item lands on its data without the seat, and its sessions look gone; `seat doctor`
  now warns about such a seat, and the Dock-window message points here.

## [0.8.2] - 2026-11-25

### Fixed
- A desktop window opened from the Dock, Finder, Spotlight or a login item on a seat's data folder (as with
  `user_data_dir = "default"`) shows the seat's login but reads Code sessions from `~/.claude`, so the seat's sessions
  and environments look gone. `seat desktop <seat>` used to call that "already running" and stop. It now names the
  seat, says nothing is lost and where the sessions are, and exits 1 with how to restart it; `seat doctor` and
  `seat desktop` say the same.

## [0.8.1] - 2026-11-25

### Changed
- The README has a full usage reference: every command and its flags, grouped by task, with examples, and a set of
  real-life walkthroughs. It replaces the short command table.

## [0.8.0] - 2026-11-24

### Added
- `--json` on `seat owner`, `list`, `ps`, `explain` and `doctor`, for orchestrators that need to ask which seat a
  folder belongs to before they start an agent. `seat owner --json` also says which seat `seat run auto` would start
  there. Every document carries `"schema": 1`; fields are only added within a version. The schema, the exit codes
  and the contract for probes, worktrees and subagents are in docs/json.md.
- A config format version: `version = 1` in config.toml (unset means 1). A config from a newer seatwise is refused
  with the version it needs, instead of being guessed at; older formats will be migrated on load.
- A weekly `host-compat` workflow runs seatwise against the latest Claude Code and Codex releases: both start on a
  guarded seat, and the `settings.json` seatwise writes is validated against Claude Code's published settings schema.
- The README says what 1.0 promises (config keys and format, commands, JSON, exit codes, the hook contract, the
  environment variables) and what seatwise protects and doesn't (it isn't a sandbox, it trusts the agent binary, it
  doesn't control the network).

### Changed
- `seat init` proposes `enforcement = "warn"` and writes `version = 1`. A config without `enforcement` still means
  `off`.
- `seat list` ends with the seats that aren't isolated (enforcement off), and `seat doctor` warns about each of them.
  A config where every seat is off is a warning now, not just information.
- Guarded seats fail closed: `seat run` puts missing or outdated hooks back before the session starts, and a `strict`
  seat whose hooks can't be written doesn't start (exit 3).
- `SEATWISE_FORCE=1` is honoured only when typed in a terminal, never inside an agent session or a hook, and it now
  only gets a start past the start checks: the hooks of that session still check it. The tool hook treats a shell
  command that sets or clears `SEATWISE_FORCE`, `SEATWISE_SEAT`, `SEATWISE_CONFIG`, `SEATWISE_REVEAL`,
  `CLAUDE_CONFIG_DIR` or `CODEX_HOME` like a reach into another seat's folder (`warn` asks, `strict` blocks).
- The environment handed to the agent drops every login and identity variable of the Claude, Anthropic, OpenAI and
  Codex families (`*_TOKEN`, `*_KEY`, `CLAUDE_CODE_ACCOUNT_UUID`, `CLAUDE_CODE_USER_EMAIL`, ...), and, when started
  from inside an agent session, all of that session's own variables.
- Hook commands carry a hook contract version (`--contract 1`), and `seat doctor` names a seat whose hooks were
  written for another one. Hooks from earlier versions show as outdated until the next start or `seat install-hooks`
  (Codex asks once to approve the changed hooks).
- `seat doctor` no longer suggests deleting a keychain login no seat uses: such a login may still be live (plain
  `claude`, another config). It now says how to make it a seat, or, for a folder that is gone, where to remove it.

## [0.7.0] - 2026-11-24

### Added
- Exit codes that say why: 1 failure, 2 usage error, 3 refused by the isolation guard (wrong or closed folder, wrong
  org), 4 no such seat, preset or group. Errors can carry a `resolution:` line with what to do about them (for example
  `Use: seat run work` when a folder belongs to another seat). Hooks still exit 2 to block, as Claude Code expects.
- Logs of what seatwise decided, in `<data dir>/logs`. Every failing command, and every `seat run`, writes a run log
  (the newest 50 are kept) and a failing command ends with `log: <path>`. Hook blocks and warnings go to `hooks.log`
  (rotated at 256 KB); `seat log` shows the latest. Lines are redacted and never hold identifiers, credentials or a
  session's arguments.

## [0.6.0] - 2026-11-24

### Changed
- Everything seatwise prints now goes through one output module, which redacts account identifiers in the process
  itself (for an agent, or when output isn't a terminal). That replaces re-running the whole command with its pipes
  filtered, so a new command can no longer leak by being left off a list; panic messages and clap's usage errors are
  redacted too. Commands that hand the terminal over (`run`, `login`, `tui`, `shell-init`, `completions`, `url`) print
  as they are. A closed pipe (`seat list | head`) no longer panics.

### Changed
- A seat, preset or group name that doesn't exist now says what it probably meant: `unknown seat 'wrk' (maybe you
  meant 'work'; known: work, frontier, personal)`. One lookup does it for every command, and the edit-distance
  helper is shared with the settings catalog's typo hints.

## [0.5.0] - 2026-11-24

### Added
- `seat share --adopt [seat]` copies the skills tools leave in `~/.claude/skills` and `~/.agents/skills` (no seat reads
  those folders) into the shared folder and adds them to the seats' `share` (every Claude Code seat if none is named).
  `seat doctor` points at it.
- `seat import projects <file>` reads an older launcher's `projects` file (`<owner> <path>` per line); the same as
  `seat link --from`, and nothing is written unless every line is valid.
- `seat doctor --desktop` lists the running desktop instances and the latest warnings and errors of the log they all
  share, each with the instances that were running when it was written (a line naming a pid goes to that instance).
- `seat doctor` notes a seat's folder that holds other seats' folders (a tool treating it as one project runs all of it
  on the outer seat; use one project per folder), and a `~/.codex` no Codex seat uses (Codex sessions that tools start
  there run without the guard), with the command that adds the seat.

## [0.4.5] - 2026-11-24

### Fixed
- The fresh-login-shell check behind `seat doctor`'s shim finding reads the shell's `PATH` instead of asking
  `command -v claude`, which answers with the `claude` function `seat shell-init` defines.

## [0.4.4] - 2026-11-24

### Fixed
- A linked git worktree outside every configured folder (Claude Code's `.claude/worktrees`, an agent tool's own
  worktree folder) belongs to the seat that owns its main checkout: `seat run auto`, `seat owner` and both hooks
  place it there, so threads in worktrees no longer need their own folder entry.
- `claude --version`, `-v`, `--help` and `-h` through the shim run in folders no seat owns (tools probe the CLI from
  their own folders): on `default_seat`, else the first enabled Claude Code seat, without the folder check. Real
  sessions there are still refused.
- Under an agent, redaction leaves the paths seatwise prints itself (config file, data folder, seat and project
  folders) intact; a UUID inside them no longer turns into `<uuid>`.
- `seat doctor` warns about `~/.claude` only when it holds settings, sessions or instructions. A folder with
  just what the desktop app or a tool left (`bridge-spawn`, `ssh`, a skill) is an info line naming its contents.
- `seat doctor` tells a shim missing from this process's PATH apart from one missing everywhere: when a fresh login shell
  finds it first, it says to restart this terminal or the desktop app instead of warning.

## [0.4.3] - 2026-11-23

### Added
- Compaction tiers: `seat compact <seat> sharp|standard|lazy` (150K tokens, the smartest; 400K; the model's whole
  window), a token count such as `250k`, or `off`; `seat compact all <tier>` sets the default for every seat. The
  tier is Claude Code's `autoCompactWindow`, enforced: kept in the seat's settings.json (so desktop app sessions get it
  too) and passed with every session seatwise starts, where it beats project and local settings. The tiers are also
  profiles (`seat run --with lazy`, a preset's `profiles`), and `seat show` and `seat tui` show a seat's tier.

## [0.4.2] - 2026-11-23

### Changed
- seatwise declares the oldest Rust it builds with: 1.95 (`rust-version`), and CI tests it there. Dependencies updated.

## [0.4.1] - 2026-11-23

### Changed
- The skills' namespace is now `seatwise`: `/seatwise:hybrid`, `/seatwise:terminal`, `/seatwise:status` and
  `/seatwise:home` (0.4.0 called the plugin `b070nd`). `seat skills install` uninstalls, disables and deletes the old
  plugin; `seat doctor` points it out.

## [0.4.0] - 2026-11-23

### Changed
- The seatwise skills are now the `b070nd` plugin on Claude seats: `/b070nd:hybrid`, `/b070nd:terminal`,
  `/b070nd:status` and `/b070nd:home`. `seat skills install` writes it into a marketplace in the seat's config folder,
  enables it in the seat's settings.json, registers it with `claude plugin install`, and removes the old loose `seat-*`
  skills. `seat doctor` (with `--fix`), `seat hybrid` and the seat card in `seat show` and `seat tui` report whether
  a seat's skills are current. Codex seats keep loose `seat-*` skills.
- `seat hybrid` works out a plan first and then carries it out. `seat hybrid --json` prints it: each step `ok`,
  `change` or `open`, and the open choices with the flag for every option, so `/b070nd:hybrid` asks only about those.
  Session-only settings (`--with`) are checked against the settings catalog, and only profiles valid for a session
  are offered; experimental ones are marked.

### Added
- The settings catalog: every Claude Code setting and environment variable (243 settings, 401 variables, from Claude
  Code's docs) with its type, allowed values, where Claude Code honors it, and whether it is experimental, plus the
  combinations the docs warn about. `seat settings search`, `seat settings show` and `seat settings check` use it.
  `seat doctor` and `seat explain` check presets and profiles against it: bad values, typos with suggestions, and
  settings Claude Code ignores where a preset is applied.
- `seat preset new` and `seat preset edit`: build or change a preset step by step in the terminal. Only settings
  Claude Code honors where the preset is applied are offered, and choices that add a problem (bad values, typos,
  documented clashes) are refused, so what it writes is valid. `seat preset set`, `unset`, `profiles` and `options`
  do the same one setting at a time, for scripts and skills.
- `seat show <seat>` and `seat projects` (both with `--json`): a seat in full, with its projects, and every project
  folder with its seat, preset, session count and last use.
- A built-in `experimental` profile that turns on Claude Code's experimental Code Mode for a session
  (`--with experimental`).

### Changed
- `seat tui` is redesigned: a list on the left and a card on the right, colored by health. Each seat card shows
  account, plan, identity, isolation, links, profile, desktop app, usage limits as bars (`u` loads them) and the
  seat's projects. A new Projects tab shows each folder's seat, preset, sessions, last use, and what a session there
  runs with. On the Presets tab, `e` opens an editor that searches the settings catalog and only takes valid choices.
- The seat views (`seat show`, `seat tui`) say "hooks outdated" rather than "hooks missing" when a seat's hooks point at another `seat` binary.
- Session presets and profiles export environment variables Claude Code reads only from the launch environment, so
  they take effect instead of being ignored in `--settings`.

## [0.3.0] - 2026-11-23

### Added
- A seat whose desktop app already runs in the app's default data folder (signed in there before seatwise, or started
  with the seat's `CLAUDE_CONFIG_DIR` by hand) is recognised: `seat init` writes `user_data_dir = "default"` for it,
  `seat doctor` suggests it and `seat doctor --fix` adds it, `seat desktop` lists the instance under that seat, and
  `seat desktop <seat>` no longer starts a second, signed-out instance next to it.

### Changed
- `.gitignore` also keeps local config copies, config backups and `.env.local` out of the repository.

## [0.2.0] - 2026-11-22

### Fixed
- seatwise ignores `XDG_CONFIG_HOME` / `XDG_DATA_HOME` when they point into a temporary folder or a session's scratch
  workspace (the desktop app's sessions set them), instead of reading and writing a throwaway config there whose path
  also contains account identifiers. `seat doctor` says when it ignored one.
- A session started from inside another agent session (`seat run`, `seat term` from a desktop Code session) no longer
  inherits that session's `ANTHROPIC_BASE_URL`, `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT` or `CLAUDE_CODE_SSE_PORT`. A base
  URL set in your own shell is kept. `CLAUDE_CODE_OAUTH_TOKEN` is cleared like the other stray credentials.
- `seat list` and `seat doctor` no longer report a seat as signed in (`IDENTITY ok`) when only its account is
  recorded: a Claude Code seat needs its login too (`.credentials.json`, or the macOS keychain item for its config
  folder). Such a seat shows `signed out (login gone)` / `SIGNED OUT`, and doctor reports it as an error.
- `eval "$(seat shell-init zsh)"` works when aliases with the same names already exist (`alias claude=...` from an
  older setup). zsh used to stop with a parse error and define nothing; the functions are now written as
  `function NAME { ... }`.

### Added
- `seat unlink --missing` removes folder entries whose folder no longer exists; it names no paths, so it works inside
  a guarded session. `seat doctor --fix` does the same.
- `seat link --from <file>` assigns many folders at once from `<owner> <path>` lines, and writes nothing unless every
  line is valid.
- `seat desktop <seat>` (macOS) starts the installed Claude desktop app on a seat: its own data folder
  (`--user-data-dir`, default `~/Library/Application Support/Claude-<seat>`, or `user_data_dir = "default"`) and the
  seat's `CLAUDE_CONFIG_DIR`, `SEATWISE_SEAT` and `BROWSER`. It never starts a second instance on the same data
  folder, and warns before a first sign-in while other instances run (macOS hands the Google sign-in answer to the
  oldest instance). `seat desktop` alone lists the running instances. Optional `[seat.desktop]` sets `app` and
  `user_data_dir`; `seat shell-init` defines `claude-desktop-<seat>` (`shell.desktop_prefix`).
- `seat doctor` reports desktop instances started without a seat (Dock or Finder), running with an outdated
  setup (restart them), or sharing a data folder.
- `seat doctor` warns when Claude Code's default `~/.claude` exists but no seat uses it (something started Claude
  Code without a seat), and when the keychain still holds a login for a config folder no seat uses, with the command
  that removes it.

### Changed
- `seat init` no longer proposes an agent's default folder (`~/.claude`, `~/.codex`) as a seat when seats with
  folders of their own exist: it is written commented out, as usually left over, and its folders are left out.
  Folders guessed from sessions in several subfolders are marked "Check it".
- seatwise is licensed under MIT only (it was MIT OR Apache-2.0), copyright b070nd. Releases up to 0.1.0 stay
  available under both licenses.
- The README covers every way to install: from crates.io (`cargo install seatwise`), from GitHub, or a prebuilt
  release binary.

## [0.1.0] - 2026-11-22

First release.

### Added
- Seats: one config folder, login and organization per Claude Code or Codex account (`agent = "codex"` gives a seat
  its own `CODEX_HOME`). `seat init` proposes a config from the config folders already on the machine, pins each
  seat's account and suggests folders from session history; temp, app-data and per-session scratch folders are left out.
- Folder ownership: each folder belongs to a seat, is a shared start folder (`any`) or is closed (`none`); the longest
  matching folder wins. `seat run auto`, `claude-<seat>` and an optional `claude` shim start the right seat.
- Isolation between seats, off by default: `warn` or `strict` hooks check prompts and file and shell tool calls, and a
  wrong-org login is caught at launch and in prompts.
- Per-seat link handling: the default browser, any browser command or profile, or the clipboard, including `/login`.
- Plans and usage: `seat profile` sizes `light`, `balanced` or `heavy` settings to the seat's plan; `seat usage` shows the
  5-hour and weekly limits used.
- Use-case presets (`explore`, `build`, `review`, `quick`, or your own) for folder groups, applied per session or written
  into the project, with a fingerprint in `SEATWISE_PRESET`. `seat explain` shows every settings layer and what it set.
- The sentinel: fixed checks for settings that cost more than they need to, on start or on a schedule (launchd,
  systemd user timer or Task Scheduler), with desktop notifications. It only reports.
- `seat tui`: turn seats, agents, presets, folder groups and the sentinel on and off in one screen.
- Desktop app and terminal side by side: `seat term` and `seat hybrid` (folder, skills, preset and a terminal on the same
  seat, in one step), and the seatwise skills for the desktop app.
- A home repo per seat, with a generated list of pointers to that seat's repos only.
- `seat doctor [--fix]`, `seat list`, `seat ps`, settings snapshots (`seat sync`), `seat add --from` (copies setup,
  never a login), shell integration for bash, zsh, fish and PowerShell.
- Codex logins by ChatGPT account or by API key (shown as "API key"; the key is never read or kept).
- Privacy: org UUIDs, org names, emails and account ids are replaced by placeholders such as `<org:work>` whenever an
  agent runs seatwise or its output is piped; hook messages never contain them, and a seat's token is only ever sent to
  Anthropic's usage endpoint.

[Unreleased]: https://github.com/b070nd/seatwise/compare/v0.9.2...HEAD
[0.9.2]: https://github.com/b070nd/seatwise/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/b070nd/seatwise/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/b070nd/seatwise/compare/v0.8.4...v0.9.0
[0.8.4]: https://github.com/b070nd/seatwise/compare/v0.8.3...v0.8.4
[0.8.3]: https://github.com/b070nd/seatwise/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/b070nd/seatwise/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/b070nd/seatwise/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/b070nd/seatwise/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/b070nd/seatwise/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/b070nd/seatwise/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/b070nd/seatwise/compare/v0.4.5...v0.5.0
[0.4.5]: https://github.com/b070nd/seatwise/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/b070nd/seatwise/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/b070nd/seatwise/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/b070nd/seatwise/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/b070nd/seatwise/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/b070nd/seatwise/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/b070nd/seatwise/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/b070nd/seatwise/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/b070nd/seatwise/releases/tag/v0.1.0
