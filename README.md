# seatwise

**Agentic development seats manager: keep your Claude Code and Codex accounts from crossing into each other's work.**

[![ci](https://github.com/b070nd/seatwise/actions/workflows/ci.yml/badge.svg)](https://github.com/b070nd/seatwise/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/seatwise.svg)](https://crates.io/crates/seatwise)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

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
- **The desktop app on a seat** (macOS): `seat desktop <seat>` (or `claude-desktop-<seat>`) starts the installed app
  with the seat's own data folder and environment, so its Code sessions are on that seat. One instance per seat, and
  `seat doctor` finds windows started from the Dock (no seat) or with an outdated setup.
- **A home repo per seat** ([the home-repo pattern](https://www.aihero.dev/the-home-repo-pattern)): one repo with the
  seat's skills, `AGENTS.md` and docs, where sessions start, and a generated list of pointers to that seat's repos only.
- **Codex seats (ChatGPT or API key):** `agent = "codex"` gives a seat its own `CODEX_HOME`, with the same folders, hooks and links.
- **Use-case presets for folder groups:** `explore`, `build`, `review`, `quick` or your own, given to folders the
  way seats are (longest path wins), applied per session or written into the project. Same folder, same settings,
  every time; `seat explain` shows every layer and what it set.
- **A sentinel** that checks, on start or on a schedule, for settings that cost more than they need to, and only
  reports.
- **`seat tui`:** one screen with a card per seat (account, plan, identity, isolation, desktop app, usage limits,
  its projects) and per project (seat, preset, sessions, last used, what a session there runs with). It also has an
  editor for preset settings that only takes valid choices, and switches for agents, groups and the sentinel.
- **Optional settings snapshots:** tracked files (e.g. `settings.json`) are checked on every start; changes are kept in
  history and `seat sync restore <seat>` brings the old version back.

Enforcement is off until you choose a level, so seatwise never blocks anything you didn't ask it to.

## Install

Any of these gives you the `seat` binary:

- **From crates.io** (released versions; needs Rust 1.95 or newer):

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

## Usage

The examples below follow one setup: a developer with a client seat `acme` (alias `work`, on the client's Team plan)
and a `personal` seat (their own Pro plan). Client repos live under `~/code/acme`, their own under `~/code/oss`.

How every command reads its arguments:

- A `<seat>` can be its name or any of its aliases (`acme` or `work`).
- A `[path]` left out means the current folder.
- Commands that take `[seats]...` act on every seat when none is named.
- `seat <command> --help` prints the flags of any command.
- Errors start with `seat:`, and most add a `resolution:` line with the command that fixes them. Exit codes: 0 done;
  1 failed (or `doctor` found an error); 2 bad usage; 3 refused by the guard; 4 no such seat, preset or group.

### Command index

| Area | Commands |
|---|---|
| [Set up seats](#set-up-seats) | `init`, `add`, `login`, `pin`, `config-path`, `enable`, `disable` |
| [Give folders to seats](#give-folders-to-seats) | `link`, `unlink`, `owner`, `import projects` |
| [Start sessions](#start-sessions) | `run`, `shell-init`, `shim`, `completions` |
| [Turn on isolation](#turn-on-isolation) | `install-hooks`, `log`, `doctor` |
| [See what's going on](#see-whats-going-on) | `list`, `show`, `projects`, `ps`, `explain`, `usage` |
| [Plans, usage and compaction](#plans-usage-and-compaction) | `profile`, `compact`, `run --with` |
| [Share skills and agents](#share-skills-and-agents) | `share`, `skills`, `home` |
| [Use-case presets](#use-case-presets) | `preset`, `group`, `settings` |
| [Desktop app and terminals](#desktop-app-and-terminals) | `desktop`, `term`, `hybrid`, `url` |
| [Keep settings in check](#keep-settings-in-check) | `sentinel`, `sync restore`, `tui` |
| [For tools and scripts](#for-tools-and-scripts) | `--json`, `hook` |

### Set up seats

| Command | What it does |
|---|---|
| `seat init [--print] [--force]` | Finds the Claude Code config folders on this machine and proposes a config: one seat per folder, each one's account, and folders taken from session history. Writes it when there's no config yet. `--print` only shows it; `--force` replaces an existing config |
| `seat add <name> [--config-dir d] [--alias a]... [--from seat] [--agent claude\|codex]` | Adds a seat. Its config folder defaults to `~/.claude-<name>`, and an existing folder is adopted as is. `--from` copies another seat's settings, `CLAUDE.md`, skills, agents, commands and plugins, never its login |
| `seat login <seat>` | Runs `claude auth login` in the seat's environment, then pins the account if none is pinned yet, and offers the usage profile that fits the plan |
| `seat pin <seat>` | Records the org and email the seat is signed into, so a later login to the wrong one is caught |
| `seat config-path` | Prints where the config file is |
| `seat enable <target>` / `seat disable <target>` | Turns a seat (`acme`), agent (`agent:codex`), preset (`preset:review`), folder group (`group:legacy`) or the `sentinel` on or off. The config stays |

A first setup on a machine that already has two Claude config folders:

```sh
seat init --print             # read the proposal first
seat init                     # write it
seat doctor                   # anything still to fix
```

A new seat for a new client, starting from the setup of an existing seat but with its own login:

```sh
seat add globex --alias gx --from acme
seat login globex             # browser sign-in; pins the org and email afterwards
```

The sign-in page opens wherever that seat's `link` setting points, so you can sign in from the client's browser
profile.

Adopting a config folder that's already there, such as a second Codex home:

```sh
seat add gpt --agent codex --config-dir ~/.codex-gpt
```

Taking a seat out of use while a contract is paused, without losing its config:

```sh
seat disable globex           # its folders stay assigned, but it can't start
seat enable globex
```

### Give folders to seats

| Command | What it does |
|---|---|
| `seat link <seat\|any\|none> [path]` | Gives a folder to a seat. `any` makes it a shared start folder (you pick the seat there); `none` closes it to every seat under `warn`/`strict` |
| `seat link --from <file>` | Many at once: one `<owner> <path>` per line, `#` comments allowed. Nothing is written unless every line is valid |
| `seat unlink [path]` | Removes a folder's entry |
| `seat unlink --missing` | Removes every entry whose folder is gone, including folders on a drive that isn't mounted. It names no paths, so it works inside a guarded session too |
| `seat owner [path] [--json]` | Which seat owns a folder, and which seat `seat run auto` starts there |
| `seat import projects <file>` | The same as `link --from`, for an older launcher's `projects` file |

The longest matching path wins, so a subfolder can belong to a different seat than its parent:

```sh
seat link acme ~/code/acme
seat link personal ~/code/oss
seat link personal ~/code/acme/forks/my-patch   # a fork you maintain yourself
seat link any ~/code/sandbox                    # throwaway experiments, either seat
seat link none ~/Documents/contracts            # no agent touches these
```

```console
$ seat owner ~/code/acme/billing
/Users/you/code/acme/billing: acme
$ seat owner ~/code/scratch
/Users/you/code/scratch: unassigned
```

Moving many folders over in one go:

```sh
cat > folders.txt <<'EOF'
# client work
acme     ~/code/acme
acme     ~/work/acme-infra
personal ~/code/oss
any      ~/code/sandbox
EOF
seat link --from folders.txt
```

Cleaning up after deleting old checkouts:

```sh
seat unlink --missing
```

### Start sessions

| Command | What it does |
|---|---|
| `seat run <seat\|auto> [args]...` | Starts Claude Code (or Codex) on a seat, in the current folder. Everything after the seat goes to the agent unchanged. `auto` picks the seat that owns the folder |
| `seat run --with <profile> <seat>` | Adds a usage profile for this one session (repeatable), passed as `--settings` |
| `seat run --home <seat>` | Starts in the seat's home repo |
| `claude-<seat>` / `claude-<alias>` | The same as `seat run <seat>`, defined by `seat shell-init`. Codex seats get `codex-<seat>`, and on macOS the desktop app gets `claude-desktop-<seat>` |
| `seat shell-init <zsh\|bash\|fish\|powershell>` | Prints those shell functions. With `shell.override_claude = true` it also defines a plain `claude` that picks the seat by folder |
| `seat shim install` / `seat shim uninstall` | A `claude` command on PATH that picks the seat by folder, for IDEs and tools that start `claude` themselves |
| `seat completions <shell>` | Shell completions (bash, elvish, fish, powershell, zsh) |

Put the shell functions in your shell's startup file once:

```sh
echo 'eval "$(seat shell-init zsh)"' >> ~/.zshrc
seat completions zsh > ~/.zfunc/_seat     # optional: tab completion
```

Then start sessions the way you'd start `claude`, with any of its flags:

```sh
cd ~/code/acme/billing
claude-acme                               # interactive session on the client seat
claude-work --resume                      # same seat, by its alias
claude-acme -p "summarize the open TODOs" # one-shot: prints the answer and exits
seat run auto                             # whichever seat owns this folder
seat run --with experimental acme         # this session only: Claude Code's experimental features
```

`--with` goes before the seat name. Anything after the name goes to Claude Code.

Starting on the wrong seat is caught before the agent starts. Under `strict` it is refused:

```console
$ cd ~/code/acme/billing && claude-personal
seat: /Users/you/code/acme/billing belongs to seat 'acme', not 'personal'.
  resolution: Use: seat run acme
```

Under `warn` the same message is printed as `seat: warning: ...` and the session starts anyway.

IDEs such as VS Code and JetBrains start `claude` themselves, so the shell functions never run. Install the shim, and
those IDEs get the right seat per project too:

```sh
seat shim install         # then put the folder it prints first on PATH
seat doctor               # confirms the shim is the `claude` found first
```

### Turn on isolation

Isolation is off until you choose a level, globally or per seat:

```toml
enforcement = "warn"        # every seat

[[seat]]
name = "acme"
enforcement = "strict"      # this seat only
```

| Level | Starting in another seat's folder | Its prompts and tool calls |
|---|---|---|
| `off` | allowed; folders only pick the seat | not checked, no hooks installed |
| `warn` | warns | prompting warns; touching another seat's files asks you first |
| `strict` | refused | blocked |

A seat signed into the wrong org always gets a warning, and under `strict` it is refused.

| Command | What it does |
|---|---|
| `seat install-hooks [seats]...` | Installs or refreshes the guard hooks in each seat's `settings.json`, or removes them where enforcement is off. `seat run` also puts missing hooks back on a guarded seat, and a `strict` seat that can't get them doesn't start |
| `seat log [-n N]` | What the hooks blocked or warned about lately (20 events by default), and where the run logs are |
| `seat doctor [--fix]` | Everything that weakens isolation or stops a seat from working; `--fix` installs or removes hooks and the shim |

```console
$ seat install-hooks
seat 'acme': hooks installed in /Users/you/.claude-acme/settings.json (enforcement: strict)
seat 'personal': hooks installed in /Users/you/.claude-personal/settings.json (enforcement: warn)
```

On a `strict` seat, the hooks stop the session from:

- reading, editing or searching another seat's folders, through Claude's own tools or a shell command;
- running a shell command that sets or clears the guard's variables, such as `SEATWISE_FORCE=1 claude -p hi` or
  `env -u CLAUDE_CONFIG_DIR ...`;
- taking any prompt while the seat is signed into the wrong org.

```sh
seat log -n 5             # the last five things the guard stepped in on
```

In your own terminal, outside any agent session, `SEATWISE_FORCE=1 seat run personal` starts a seat in a folder it
doesn't own, for the rare times you mean to. Inside an agent session, a hook, or a script without a terminal, the
variable is ignored. The hooks still check every prompt and tool call of a forced session.

Doctor checks for:

- missing or stale hooks;
- wrong or unpinned accounts, and two seats on one account;
- `CLAUDE.md` leaking in from a parent folder;
- stray API keys;
- a seat's folder that holds other seats' folders;
- an unguarded `~/.codex`;
- seats with enforcement off next to guarded ones.

```console
$ seat doctor
error  seat 'acme': its hooks are from a seatwise before hook contracts (this one speaks 1) [--fix]
$ seat doctor --fix
```

`seat doctor --fix` repairs every finding marked `[--fix]`. Doctor exits 1 while any error is left, so it also works as
a check in a script.

### See what's going on

| Command | What it does |
|---|---|
| `seat list [--json]` | Every seat: config folder, account, plan, identity check, enforcement, hooks, link mode, profile, folders |
| `seat show <seat> [--json]` | One seat in full: account, plan, identity, isolation, links, desktop app, and its projects |
| `seat projects [--json]` | Every project folder: its seat, preset, sessions and when it was last used |
| `seat ps [--json]` | The agent sessions running now, by seat, with each one's settings fingerprint |
| `seat explain [path] [--seat s] [--with p] [--json]` | What a session in a folder would run with: group, preset, each settings layer, and which layer set each value |
| `seat usage [seats]...` | How much of each seat's 5-hour and weekly limits is used (see [Plans and usage](#plans-and-usage)) |

```console
$ seat list
SEAT         CONFIG              ACCOUNT  PLAN           IDENTITY  ENFORCE  HOOKS  LINKS    PROFILE   FOLDERS
acme (work)  ~/.claude-acme      ...      Team standard  ok        strict   ok     firefox  balanced  2
personal     ~/.claude-personal  ...      Pro            ok        warn     ok     system   light     2
```

In the `IDENTITY` column:

- `ok`: the seat is signed into its pinned org and email.
- `WRONG`: it is signed into a different one.
- `not pinned`: `seat pin` hasn't run yet.

`HOOKS` shows `MISSING` or `stale` when the guard can't do its job.

Before you start a long task, check which sessions are already running and on which seat:

```sh
seat ps
```

When a session runs with a model or permission mode you didn't expect, ask where it came from:

```sh
seat explain ~/code/acme/billing
```

It prints every settings layer in order (yours, the project's, the preset's, seatwise's) and, for each value, the layer
that set it.

Before a long refactor, check how much of this week's limit is left:

```sh
seat usage acme
```

### Plans, usage and compaction

| Command | What it does |
|---|---|
| `seat profile <seat> [name] [--usage]` | Shows the seat's plan and the usage profile that fits it (`light`, `balanced`, `heavy`, or your own); with a name, applies it to the seat's `settings.json`. `--usage` also weighs this week's usage |
| `seat compact [seat\|all] [sharp\|standard\|lazy\|<tokens>\|off]` | When Claude Code compacts the conversation, enforced per seat. Without a value, shows it |

```sh
seat profile personal                 # Pro: suggests light
seat profile acme heavy               # the client is on Team premium: use it
seat profile acme --usage             # one step lighter when this week is mostly used
seat compact acme sharp               # compact at 150K tokens, the sharpest
seat compact all standard             # every seat that sets none of its own
seat compact personal 250k
seat run --with lazy acme             # one long session that compacts as late as it can
```

More in [Plans and usage](#plans-and-usage) and [Compaction tiers](#compaction-tiers).

### Share skills and agents

| Command | What it does |
|---|---|
| `seat share [seats]...` | Copies each seat's `share` items (skills, agents, commands) from the shared folder now. Also done on every start |
| `seat share --adopt [seats]...` | First moves the skills that tools leave in `~/.claude/skills` and `~/.agents/skills`, which no seat reads, into the shared folder, and adds them to the seats' `share` |
| `seat skills install [seats]...` | Installs the seatwise skills as the `seatwise` plugin: `/seatwise:hybrid`, `/seatwise:terminal`, `/seatwise:status`, `/seatwise:home` |
| `seat skills show <name>` | Prints one of those skills as it would be installed |
| `seat home init <seat> [path]` | Creates or adopts the seat's home repo (its skills, `AGENTS.md`, and pointers to its other repos) and gives it to the seat |
| `seat home sync [seats]...` | Rewrites the home repo's "Other repos" pointers from the seat's folders |

A code-review skill you wrote once, used by both seats:

```toml
[[seat]]
name = "acme"
share = ["skills/code-review", "agents"]
```

```sh
seat share                    # copy it in now instead of on the next start
```

A skill installer dropped files into `~/.claude/skills`, where no seat looks:

```sh
seat share --adopt
```

A home repo, so one session on the client seat knows about all of the client's repos:

```sh
seat home init acme ~/code/acme/home
seat run --home acme
```

### Use-case presets

| Command | What it does |
|---|---|
| `seat preset list` / `seat preset show <name>` | Every preset (the built-in `explore`, `build`, `review` and `quick`, and your own), and one preset's resolved settings |
| `seat preset new <name> [--from p] [--description d]` | Builds a preset step by step; only valid choices are taken |
| `seat preset edit <name>` | Changes one the same way, keeping comments in the config |
| `seat preset set <name> <key> <value>` / `seat preset unset <name> <key>` | Changes one setting or environment variable; refused when Claude Code wouldn't take it there |
| `seat preset profiles <name> [profiles]...` | The usage profiles a preset builds on, in order; none clears them |
| `seat preset options <name> [words]... [--experimental]` | What the preset can take now, with values |
| `seat group add [path] [--name n] [--preset p\|none] [--apply session\|project] [--sources user,local]` | Gives a folder a preset. On a terminal it asks for what isn't given |
| `seat group list` / `seat group remove <name>` | Folder groups |
| `seat settings search [words]... [--experimental] [--env] [--all]` | Every Claude Code setting and environment variable, from the settings catalog |
| `seat settings show <key>` | What a setting does, where Claude Code honors it, and how to set it in a preset |
| `seat settings check [name] [--file f] [--target session\|local\|user]` | Checks a preset, profile or settings JSON file against the catalog |

In client repos where you mostly review pull requests, start sessions in plan mode with the heavy profile:

```sh
seat group add ~/code/acme --name acme-review --preset review
seat explain ~/code/acme/billing      # confirm: preset review, permission mode plan
```

For one legacy repo inside it whose committed `.claude/settings.json` you don't trust:

```sh
seat group add ~/code/acme/legacy --name legacy --preset none --sources user,local
```

Your own preset for infrastructure work, built one setting at a time from a script:

```sh
seat preset new ops --from build --description "Infra: asks before every edit"
seat preset set ops permissions.defaultMode default
seat preset options ops effort        # which effort values it can take
seat settings check ops
```

Finding a setting you half remember:

```sh
seat settings search thinking
seat settings show CLAUDE_CODE_EXPERIMENTAL_CODE_MODE
```

More in [Presets and folder groups](#presets-and-folder-groups).

### Desktop app and terminals

| Command | What it does |
|---|---|
| `seat desktop [seat]` | Starts the Claude desktop app on a seat (macOS), with its own data folder, unless it already runs. Without a seat, lists the running instances |
| `seat desktop <seat> --own-folder` | Moves a seat's desktop data out of the app's usual folder into `Claude-<seat>` (the app must be quit), so a Claude opened from the Dock can't land on it |
| `seat doctor --desktop` | The desktop instances running, and recent warnings and errors from their shared log, each with the instances that were running then |
| `seat term [--seat s] [--dir d] [--with p] [--home] [--print] [-- args]` | Opens a terminal window running the seat's CLI in a project. Inside a session it takes that session's seat and project |
| `seat hybrid [--seat s] [--dir d] [--link] [--preset p] [--with p] [--check] [--no-open] [--print] [--json]` | Sets up the CLI side of a project next to the desktop app (folder, skills, preset), then opens a terminal on the same seat |
| `seat url [--copy] <url>` | Opens a link the way the current seat's `link` setting says. Inside sessions, `BROWSER` points here |

The client's desktop app and your own, side by side, each with its own login:

```sh
seat desktop acme
seat desktop personal
seat desktop                  # which instances are running now
```

When you're chatting in the desktop app and need the CLI for a long test run on the same project:

```sh
seat hybrid --check                   # what's set up already, changing nothing
seat hybrid --link --preset build     # give it to the seat, add a preset, open the terminal
seat term --print                     # just print the command, for your own terminal app
```

Inside the desktop app, `/seatwise:terminal` or `/seatwise:hybrid` does the same. More in
[Desktop app, terminal and home repos](#desktop-app-terminal-and-home-repos).

### Keep settings in check

| Command | What it does |
|---|---|
| `seat sentinel [seats]... [--here] [--notify] [--quiet]` | Looks for settings that cost more than they need to. It reports and never changes anything |
| `seat sentinel rules` | The rules it checks |
| `seat sentinel install [--dry-run]` / `seat sentinel uninstall` | Runs it every `sentinel.interval` in the background (launchd, a systemd user timer, or Task Scheduler) |
| `seat sync restore <seat>` | Swaps the files listed in `track` back to their previous snapshot; run it again to undo |
| `seat tui` | Full-screen settings: seats, agents, presets, folder groups and the sentinel. `w` writes, keeping comments |

```sh
seat sentinel --here                  # just this project and its seat
seat sentinel install --dry-run       # see the schedule before installing it
```

Sessions ran `/model` and answered "always allow" one too many times, and the seat's `settings.json` drifted. With
`track = ["settings.json", "CLAUDE.md"]` in the config, every start snapshots those files and reports a change:

```sh
seat sync restore acme                # back to the last snapshot
seat sync restore acme                # changed your mind: swap again
```

### For tools and scripts

`owner`, `list`, `ps`, `explain` and `doctor` take `--json` for orchestrators and scripts that start agent sessions
themselves. The schema, the exit codes and how an orchestrator should start sessions are in [docs/json.md](docs/json.md).

```sh
# which seat should a worker start in this checkout?
seat owner ~/code/acme/billing --json

# a scheduled check that the setup is still sound
seat doctor --json > doctor.json || echo "seatwise found problems"
```

`seat hook prompt` and `seat hook tool` are the guard hooks themselves. `install-hooks` writes them into each seat's
`settings.json` and Claude Code runs them, so you never call them by hand.

## Real-life examples

**A second client starts Monday.** You want its sessions to never see the first client's code:

```sh
seat add globex --from acme           # same skills and settings, its own login
seat login globex
seat link globex ~/code/globex
git clone https://git.example.com/globex/api.git ~/code/globex/api
```

Then add `enforcement = "strict"` to the globex seat in the config:

```sh
seat install-hooks globex
cd ~/code/globex/api && claude-globex
```

**You typed `claude-personal` in a client repo out of habit.** Under `strict` the session doesn't start, and the error
names the right command (`Use: seat run acme`). Under `warn` it starts with a warning, and the session asks you first
before touching any file in another seat's folders.

**Your browser had the wrong account open when you signed in.** The seat is now signed into your personal org:

```console
$ seat list
SEAT         ...  IDENTITY  ENFORCE  ...
acme (work)  ...  WRONG     strict   ...
```

Under `strict` the seat refuses to start, and its hooks block prompts until you fix it:

```sh
seat login acme          # sign in again, this time to the client's org
seat list                # IDENTITY: ok
```

**Your IDE's Claude panel used the wrong subscription.** The IDE starts `claude` from PATH, not through your shell:

```sh
seat shim install
seat doctor              # check that the shim is the `claude` found first
```

**An agent orchestrator runs workers across your repos.** Before each worker starts, the orchestrator asks seatwise
which seat owns the checkout and starts that seat:

```sh
seat owner "$CHECKOUT" --json          # read "run_seat"
seat run "$SEAT" -p "$TASK"            # exit 3: refused by the guard; don't retry on another seat
```

**You're leaving a client.** Keep the config for invoices and questions, but make sure nothing can start on it:

```sh
seat disable acme
seat link none ~/code/acme             # closed to every seat; or remove the entry: seat unlink ~/code/acme
```

**Something feels off.** Start with these three:

```sh
seat doctor              # setup problems, each with its fix
seat log                 # what the guard stepped in on
seat explain             # why this folder's sessions run the way they do
```

## Plans and usage

`seat list` shows each seat's plan (Pro, Max 5x/20x, Team standard/premium, Enterprise) and `+extra` when usage beyond
the plan is billed. `seat pin` records the plan, and `seat doctor` reports when it changes.

A usage profile is a few settings merged into the seat's `settings.json` (never its hooks): main model, subagent model,
effort and extended thinking. `seat profile <seat>` suggests the one that fits the plan, one step lighter when this
week's limit is mostly used (`--usage`). Define your own as `[profile.<name>]` tables in the config.

### Compaction tiers

How much context a session carries before Claude Code compacts it is its `autoCompactWindow` setting. Less old
context keeps the model sharper; more means fewer compactions. seatwise names three tiers and enforces the one you
pick:

| Tier | Compacts at | For |
|---|---|---|
| `sharp` | 150K tokens | the smartest mode: the least old context in the way |
| `standard` | 400K tokens | the default tier |
| `lazy` | the model's whole window | as few compactions as possible |

```sh
seat compact work sharp       # this seat; also: a token count such as 250k, or off
seat compact all standard     # every seat that sets none of its own
seat compact                  # what each seat gets
seat run --with lazy work     # this one session only
```

An enforced tier is kept in the seat's `settings.json` (so desktop app sessions get it too, and a change made there
is put back on the next start) and passed with every session seatwise starts, where it beats project and local
settings. Claude Code caps the window at the model's own, so on a 200K model every tier above it compacts at 200K.
The tiers are profiles too: a preset can list one in its `profiles`.

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

### The settings catalog

seatwise ships a catalog of every Claude Code setting and environment variable (from Claude Code's settings and
env-var docs): its type, allowed values, where Claude Code honors it, and whether it's experimental. It also includes
the combinations the docs warn about.

```sh
seat settings search --experimental    # what's new to try
seat settings search thinking          # every setting about thinking
seat settings show CLAUDE_CODE_EXPERIMENTAL_CODE_MODE
seat settings check my-preset --target local
```

`seat doctor` and `seat explain` check presets and profiles against it. That catches values Claude Code doesn't
accept (`effortLevel = "hihg"`), typos (`efortLevel`: did you mean `effortLevel`?), and settings Claude Code ignores
where the preset lands. Code Mode, for example, works from `--settings` (a `session` preset or `--with`) but not from
`.claude/settings.local.json` (an `apply = "project"` preset). seatwise exports variables Claude Code reads only from
the launch environment itself, so they work in session presets too. Keys the catalog doesn't know get a warning, never
an error, so a new Claude Code release doesn't break your config.

### Building your own presets

`seat preset new <name>` (optionally `--from build`, or from a profile) walks you through a preset in the terminal. You
can search the catalog, list experimental settings, pick values from lists, choose profiles, and see the merged result
before writing. It only offers settings Claude Code honors where the preset will be applied, and it refuses a choice
that adds a problem: a value Claude Code doesn't take, a typo, or a documented clash (`CLAUDE_CODE_EFFORT_LEVEL` next to
`effortLevel`, say). So whatever it writes is valid. `seat preset edit <name>` changes one the same way, comments in the
config included.

For scripts and skills, the same checks run one setting at a time:

```sh
seat preset options fast effort          # what the preset can take now, with values
seat preset set fast CLAUDE_CODE_EXPERIMENTAL_CODE_MODE 1
seat preset unset fast effortLevel
seat preset profiles fast balanced
```

The built-in `experimental` profile turns on Claude Code's newest experimental features for one session:
`seat run <seat> --with experimental`, or `seat hybrid --with experimental` next to the desktop app. Today that's
Code Mode: Claude writes a short script that batches tool calls, and each call still goes through your permission
rules and seatwise's hooks.

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
shell), ask for a terminal. `/seatwise:terminal` runs `seat term`, which opens one on the same seat and project:

```sh
seat skills install            # every seat: /seatwise:hybrid, /seatwise:terminal, /seatwise:status, /seatwise:home
seat hybrid --check            # what is set up for this project, changing nothing
seat hybrid --link --preset build --with experimental
seat term --with experimental  # just the terminal, from any shell
```

`seat hybrid` sets up the command-line side in one step, then opens the terminal. It makes sure the project folder
belongs to the session's seat (`--link` assigns an unassigned one), installs the seat's skills, gives the project a
use-case preset (`--preset`), and opens a terminal with session-only settings (`--with`). It refuses a folder that
belongs to another seat. The session-only settings are checked against the settings catalog first: a profile that
gives settings Claude Code would not take in a session is refused before anything changes.

In the desktop app, run `/seatwise:hybrid` or ask for "hybrid mode" or "the CLI for this project". The skill reads the
plan (`seat hybrid --json`: each step `ok`, `change` or `open`, and the open choices with the flag for each option)
and asks you only about the open choices: whether the folder should belong to the seat, a preset, and session-only
settings such as `--with experimental` (Code Mode), offering only profiles that are valid for a session.

On Claude seats the skills come as a plugin named `seatwise`, so they share one namespace and stay out of the way of
your own skills. It lives in a small marketplace in the seat's config folder (`seatwise-plugins/`), is enabled in the
seat's settings.json (`extraKnownMarketplaces` and `enabledPlugins`) and registered with `claude plugin install` in
that seat. Claude Code reads it in place: after an upgrade, `seat skills install` updates it for the next session, or
`/reload-plugins` in a running one. The loose `seat-*` skills of seatwise 0.3 and earlier are removed (folders you
changed are left alone), and `seat doctor` notices outdated skills. Codex has no plugins: its seats keep loose
`seat-hybrid`, `seat-terminal`, `seat-status` and `seat-home` skills.

`seat term` takes the seat from the session's config folder (or the org the desktop app exports, when exactly one seat
is pinned to it) and the project from `CLAUDE_PROJECT_DIR`. `terminal.profile` sets the profile it uses by default.

### The desktop app on a seat (macOS)

```sh
seat desktop work            # or claude-desktop-work after `seat shell-init`
seat desktop                 # which instances run, on which seat, since when
```

Every seat runs the same installed `/Applications/Claude.app` (no copies), with its own data folder
(`~/Library/Application Support/Claude-<seat>`: the desktop's own login and settings) and the seat's
`CLAUDE_CONFIG_DIR`, `SEATWISE_SEAT` and `BROWSER`, so its Code sessions get the seat's hooks, folders and links. A
seat that already uses the app's usual folder keeps it with `user_data_dir = "default"` in `[seat.desktop]`.

**Already signed in to the desktop app?** Its login lives in the app's usual folder, not in `Claude-<seat>`. When the
app runs there for one of your seats (started with that seat's `CLAUDE_CONFIG_DIR`, from the Dock for a seat in
`~/.claude`, or by an older setup), `seat init` writes `user_data_dir = "default"` for that seat, `seat doctor`
suggests it (`seat doctor --fix` adds it), and `seat desktop <seat>` won't start a second, signed-out instance next to
it. Only one seat can have the usual folder.

That keeps the login, but the usual folder is also where every Claude opened from the Dock, Finder, Spotlight or a
login item lands, without the seat. Such a window shows the seat's desktop login but reads Code sessions from
`~/.claude`, so the seat's sessions and environments look gone (they aren't: they are in the seat's config folder).
Move the seat out once, with the app quit:

```sh
seat desktop work --own-folder   # moves the usual folder to Claude-work, login and sessions included
seat desktop work
```

After that, a Dock launch opens a new, signed-out window that can't hide anything. `seat doctor` reminds you while a
seat still uses the usual folder, and `seat desktop <seat>` refuses to call a Dock-opened window on its folder
"already running".

- **One instance per seat.** If the seat's instance already runs, `seat desktop` says so instead of starting a second
  one on the same data folder.
- **Signing in while other instances run.** A Google sign-in answers through a `claude://` link, which macOS hands to
  the *oldest* running Claude; that one ignores it, and the new seat keeps waiting on its login page. Sign in with
  your email (you type the code into the app), or quit the other Claude windows until the new one is signed in.
  `seat desktop` reminds you when a seat starts for the first time while others run.
- **Starting from the Dock or Finder** gives no seat: the app uses `~/.claude` (no hooks) and its usual data folder.
  `seat doctor` reports such instances, and `~/.claude` coming back; on a seat's folder, see `--own-folder` above.
- **A window that keeps coming back maximized.** The app saves a window as maximized whenever it fills a screen's
  usable area (on a screen without a menu bar or Dock, that's the whole screen), and then maximizes it on every start.
  It also moves a window to the main screen when it starts before it sees the others. Give the seat a size, and every
  `seat desktop` start opens with it, never maximized or full screen:

  ```toml
  [seat.desktop]
  window = "1800x1900"    # width x height in points
  ```

  The saved position is kept while the window still fits on that screen. A window you resize keeps that size until
  the next start; change `window` to keep a new one.
- **An instance keeps the environment it started with.** After changing a seat's link handling or config folder,
  `seat doctor` says which desktop to restart.
- Deleting a seat's data folder signs that desktop seat out.
- All instances write to one `~/Library/Logs/Claude/main.log`; `seat desktop` lists their start times to match lines.

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

## Good to know

- **Managing other seats from inside a seat's session.** With enforcement on, a session on one seat can't name another
  seat's folders, read seatwise's own repository, or touch folders closed to all seats; that includes `seat unlink
  <other seat's path>`. Change other seats' setup from your own terminal. Commands that name no paths still work,
  such as `seat unlink --missing`, `seat list` and `seat doctor`.
- **Exit codes** say why a command failed: 1 failure, 2 usage error, 3 refused by the isolation guard (wrong or closed
  folder, wrong org), 4 no such seat, preset or group. An error may add a `resolution:` line with what to do.
- **Logs.** Every `seat run` and every failing command writes a run log in `<data dir>/logs` (the newest 50 are kept;
  a failure ends with `log: <path>`). Hook blocks and warnings go to `hooks.log`; `seat log` shows the latest. Lines
  hold folder paths and seat names, redacted like all output, and never a session's arguments.
- **Starting another seat from inside a session** is refused under `strict` when the current folder belongs to the
  current seat: `claude-<other>` checks the folder the same way. Start it from that seat's folder or a shared start
  folder.
- **The first start of each seat** asks Claude Code's one-time questions (trusting the folder, the theme) again: every
  seat has its own config folder.
- **Terminals that were already open** keep their old functions and aliases. Run `eval "$(seat shell-init zsh)"` again
  there, or open a new one.
- **The desktop app's sessions** may point `XDG_CONFIG_HOME` into their scratch workspace; seatwise ignores such
  folders and keeps using `~/.config/seatwise`.

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

## What 1.0 promises

From 1.0 on, these change only with a new major version:

- **Config.** The keys in [config.example.toml](config.example.toml) and what they mean. The file carries its format
  (`version = 1`): seatwise migrates an older one when it reads it and refuses one from a newer seatwise.
- **Commands.** The commands, flags and arguments in the table above. The human-readable output is not frozen: tools
  should read `--json`.
- **JSON.** The documents in [docs/json.md](docs/json.md) (`"schema": 1`): fields are only added.
- **Exit codes.** 0 done, 1 failed, 2 usage error, 3 refused by the guard, 4 no such seat, preset or group.
- **The hook contract** (contract 1): the `UserPromptSubmit` and `PreToolUse` hooks, the command
  `seat hook prompt|tool <seat> --contract 1`, exit 2 to block, JSON on stdout to warn or ask. A new contract can come
  with a minor release; `seat run` then puts the hooks back, and `seat doctor` names seats still on the old one.
- **Environment.** The variables seatwise reads (`SEATWISE_CONFIG`, `SEATWISE_DATA`, `SEATWISE_FORCE`,
  `SEATWISE_REVEAL`, `SEATWISE_CLAUDE`, `SEATWISE_CODEX`) and sets for a session (`SEATWISE_SEAT`, `SEATWISE_PRESET`).

Not covered: the TUI, the sentinel's rules, the built-in presets and usage profiles, the settings catalog, and the
desktop app support, which follows whatever the Claude desktop app does and is best effort.

## Threat model

seatwise keeps accounts, orgs and folders out of each other's work. It is built against mistakes and against an agent
wandering off: a session on the wrong account, in the wrong folder, or reaching into another seat's files, and an
agent trying to switch seats or lift the guard from inside its session.

It does not protect against:

- **A hostile program.** It is not a sandbox. Pair it with Claude Code's sandbox for a hard boundary.
- **The agent binary.** seatwise trusts the `claude` and `codex` it finds on `PATH` (or in `SEATWISE_CLAUDE` /
  `SEATWISE_CODEX`). It doesn't verify them.
- **The network.** Sessions can reach any host the agent can.
- **Anyone who can edit** seatwise's config, a seat's settings files, or the `seat` binary. They can change or remove
  the hooks. A guarded seat puts them back on its next start, but a running session isn't watched.
- **Shell commands that hide paths**, such as relative paths and paths built at runtime (see the limits above).
- **Changes in the hosts.** The hook contract, `CLAUDE_CONFIG_DIR` handling and the desktop app's data folders belong
  to Claude Code, Codex and the desktop app, and can change without notice. A weekly CI job (`host-compat`) runs
  seatwise against their latest releases.

The details are in [docs/security.md](docs/security.md).

## Platforms

| | macOS | Linux | Windows |
|---|---|---|---|
| Launcher, folder and org guard, hooks, settings snapshots | yes | yes | yes |
| Per-seat link handling (browser command, copy) | yes | yes (copy needs `wl-copy` or `xclip`) | yes |
| Desktop app seats (`seat desktop`) | yes | n/a (no desktop app) | planned |

## More

- [docs/architecture.md](docs/architecture.md): how the pieces fit, and the module map.
- [docs/security.md](docs/security.md): what seatwise protects, and what it does not.
- [CHANGELOG.md](CHANGELOG.md), [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md).

## License

MIT © 2026 seatwise. See [LICENSE](LICENSE).
