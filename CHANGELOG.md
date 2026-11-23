# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/b070nd/seatwise/compare/v0.4.3...HEAD
[0.4.3]: https://github.com/b070nd/seatwise/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/b070nd/seatwise/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/b070nd/seatwise/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/b070nd/seatwise/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/b070nd/seatwise/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/b070nd/seatwise/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/b070nd/seatwise/releases/tag/v0.1.0
