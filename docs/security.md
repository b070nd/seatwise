# Security model

seatwise keeps several agent accounts ("seats") from crossing into each other's work. It is a guard against
mistakes, such as a session on the wrong account or in the wrong folder, not a sandbox against a hostile process.

## What it protects

- **Folders.** With enforcement on (`warn` or `strict`), the `PreToolUse` hook checks every file tool call (Read, Edit,
  Write, Glob, Grep, LS, NotebookEdit) exactly, including nested folders and symlinks, and scans shell commands for
  absolute and home-relative paths. `warn` asks you first; `strict` blocks.
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
