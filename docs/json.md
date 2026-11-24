# JSON output and the orchestrator contract

Tools that start agent sessions themselves (orchestrators such as bb, IDEs, CI jobs, agent frameworks) can ask
seatwise which seat a folder belongs to before they start `claude` or `codex`, and read the state of every seat.

## Commands

| Command | Answers |
|---|---|
| `seat owner [path] --json` | Who owns a folder, and which seat `seat run auto` would start there |
| `seat list --json` | Every seat: agent, config folder, sign-in and identity state, enforcement, hooks, folders |
| `seat ps --json` | The agent sessions running now, by seat |
| `seat explain [path] --json` | What a session in a folder would run with: group, preset, settings layers, effective values |
| `seat doctor --json` | The findings of `seat doctor`; the exit code is the same as without `--json` |

`seat show <seat> --json` and `seat projects --json` exist too. They are older and carry no `schema` field yet.

## Stability

- Every document is one JSON object on stdout with `"schema": 1`.
- Within a schema version, fields are only added. Renaming or removing a field, or changing what it means, raises the
  version. Ignore fields you don't know.
- Notes and warnings go to stderr, so stdout stays one document.
- Account identifiers (org UUIDs and names, emails, account ids) are never part of these documents. The usual
  redaction applies on top, for an agent or when output isn't a terminal.
- Paths are absolute.

## Documents

`seat owner --json`:

```json
{
  "schema": 1,
  "path": "/home/me/work/api",
  "kind": "seat",
  "owner": "work",
  "run_seat": "work",
  "run_agent": "claude",
  "enforcement": "strict"
}
```

- `kind` is `seat`, `any` (a shared start folder), `none` (closed to every seat) or `unassigned`.
- `owner` is the seat's name when `kind` is `seat`, else `null`.
- `run_seat` is the seat `seat run auto` starts here without asking. It is `null` when seatwise would ask on a
  terminal or refuse: a shared folder, a disabled owner, or an unassigned folder without `default_seat` or under
  `strict`.

`seat list --json`: `{"schema": 1, "seats": [...]}`. Each seat has:

- `name`, `aliases`, `agent` (`claude` or `codex`), `enabled`, `config_dir`.
- `signed_in`, `identity` (`ok`, `wrong`, `unpinned`, `signed_out`, `logged_out`), `pinned`, `plan`.
- `enforcement` (`off`, `warn`, `strict`), `isolated` (enforcement is on and the hooks are current).
- `hooks` (`current`, `missing`, `outdated`), `hook_contract` (the contract its installed hooks were written for, or
  `null`).
- `link`, `profile`, `folders`.

`seat ps --json`: `{"schema": 1, "sessions": [{"seat", "pid", "minutes", "preset", "cwd"}]}`. `seat` is the seat
name, or the config folder in parentheses when a session runs outside any seat.

`seat explain --json`: `seat`, `agent`, `project`, and for Claude Code seats `group`, `preset`, `preset_note`,
`apply`, `sources`, `fingerprint` (the same as `SEATWISE_PRESET` in a session seatwise started), `layers`
(`name`, `path`, `exists`), and `effective` (`key`, `value`, `layer`).

`seat doctor --json`: `ok` (no errors), `config`, `config_format` (the config format this seatwise reads),
`hook_contract` (the hook contract it installs), and `findings` (`level`: `error`, `warn` or `info`; `message`;
`fixable`, which is what `seat doctor --fix` would handle).

## Exit codes

These are stable from 1.0:

| Code | Meaning |
|---|---|
| 0 | Done |
| 1 | Failed (and `seat doctor`: an error was found) |
| 2 | Usage error (unknown command or flag) |
| 3 | Refused by the isolation guard: wrong or closed folder, wrong org, a guarded seat without its hooks |
| 4 | No such seat, preset or group |

The hooks Claude Code and Codex run (`seat hook prompt|tool`) exit 2 to block, as the hosts expect.

## Starting sessions from an orchestrator

- **Which seat.** Run `seat owner <folder> --json` and start on `run_seat`, or run `seat run auto` in the folder and
  let seatwise choose. If `run_seat` is `null`, ask the user or give the folder a seat (`seat link <seat> <folder>`).
- **Probes.** `claude --version` and `--help` (through the shim, or `seat run auto --version`) run on a seat even in
  a folder no seat owns, because they start no session.
- **Worktrees.** A git worktree outside the project belongs to the seat that owns its main checkout.
- **Subagents and background sessions** that the agent starts itself inherit the session's seat. seatwise strips no
  variable from them, and their tool calls go through the same hooks.
- **Never** set `SEATWISE_FORCE`, `SEATWISE_SEAT` or `CLAUDE_CONFIG_DIR` yourself to choose a seat: the guard treats
  that as a bypass, and `SEATWISE_FORCE` only counts when a person types it in a terminal.
