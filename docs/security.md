# Security model

seatwise keeps several agent accounts ("seats") from crossing into each other's work. It is a guard against
mistakes, such as a session on the wrong account or in the wrong folder, not a sandbox against a hostile process.

## What it protects

- **Folders.** With enforcement on (`warn` or `strict`), the `PreToolUse` hook checks every file tool call (Read, Edit,
  Write, Glob, Grep, LS, NotebookEdit) exactly, including nested folders and symlinks, and scans shell commands for
  absolute and home-relative paths. `warn` asks you first; `strict` blocks.
- **Fail closed.** A guarded seat starts only with its hooks in place. Missing or outdated hooks (another `seat`
  binary, seat name or hook contract) are put back before the session starts; a `strict` seat whose hooks can't be
  put back doesn't start.
- **No way out from inside a session.** `SEATWISE_FORCE=1` gets one start past the start checks (to run `/login` on a
  seat signed into the wrong org), and only when typed in a terminal: inside an agent session, in a hook, or without
  a terminal it is ignored, and the hooks of a forced session still check it. The tool hook treats a shell command
  that sets or clears `SEATWISE_FORCE`, `SEATWISE_SEAT`, `SEATWISE_CONFIG`, `SEATWISE_REVEAL`, `CLAUDE_CONFIG_DIR` or
  `CODEX_HOME` (`VAR=1 claude`, `env -u`, `export`, `unset`, `bash -c '...'`, PowerShell's `$env:`) like a reach
  into another seat's folder.
- **Environment.** Logins and identities from whatever started seatwise (`ANTHROPIC_API_KEY`, `*_TOKEN`,
  `CLAUDE_CODE_ACCOUNT_UUID`, `CLAUDE_CODE_USER_EMAIL`, `CLAUDE_CODE_ORGANIZATION_UUID`, ...) never reach the agent,
  nor the other agent's config folder. Started from inside an agent session (a desktop Code session, for one), none
  of that session's wiring (`CLAUDECODE`, `CLAUDE_CODE_*`, `ANTHROPIC_*`, `CODEX_*`) does either. Settings you export
  in your own shell, such as `ANTHROPIC_MODEL`, pass when you start from that shell.
- **Organizations.** A seat can be pinned to an org and email. A different login is reported at launch and, with
  enforcement on, in prompts.
- **Links.** A seat's links, including `/login`, can go to a chosen browser or profile, or to the clipboard.
- **Account identifiers.** Org UUIDs, org names, emails and account ids never leave the computer through seatwise:
  - Whenever an agent runs seatwise, or its output is piped, they are replaced by placeholders such as `<org:work>`.
  - Hook messages never contain them.
  - `SEATWISE_REVEAL=1` only works outside an agent.
- **Logs.** The run logs and `hooks.log` in the data folder hold folder paths and seat names, redacted like all
  output, and never identifiers, credentials or a session's arguments.
- **Tokens.** seatwise never prints, stores or refreshes a login token. `seat usage` sends a seat's token only to
  Anthropic's usage endpoint, the host Claude Code itself sends it to, and refuses any other host.

## What it does not protect against

- Shell commands that reach another seat's folder through relative paths (`../other`) or paths built at runtime. Use
  Claude Code's sandbox if you need a hard boundary.
- Programs other than the agent sessions, and you yourself, opening any folder.
- Anyone who can edit seatwise's config file, the agent's settings files, or the `seat` binary: they can change or turn
  off the hooks.
- A compromised account or host.

## Defaults

- Enforcement is off until you choose a level.
- Links go to the system default.
- Nothing is scheduled.
- The `claude` override and the shim are opt-in.
- A per-repository file naming a seat is not supported on purpose: anyone can commit one, and it would let a cloned
  repository choose its own seat.
