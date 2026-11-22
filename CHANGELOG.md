# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- `seat list` and `seat doctor` no longer report a seat as signed in (`IDENTITY ok`) when only its account is
  recorded: a Claude Code seat needs its login too (`.credentials.json`, or the macOS keychain item for its config
  folder). Such a seat shows `signed out (login gone)` / `SIGNED OUT`, and doctor reports it as an error.
- `eval "$(seat shell-init zsh)"` works when aliases with the same names already exist (`alias claude=...` from an
  older setup). zsh used to stop with a parse error and define nothing; the functions are now written as
  `function NAME { ... }`.

### Added
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

[Unreleased]: https://github.com/b070nd/seatwise/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/b070nd/seatwise/releases/tag/v0.1.0
