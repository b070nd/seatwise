---
name: seat-home
description: Work across all of this seat's repos from its home repo, using the pointers in the home repo's AGENTS.md. Use when a task spans several repos, when the user mentions another of their projects by name, or when a repo listed in "Other repos" is missing locally.
---

# Work from the seat's home repo

The home repo's AGENTS.md has an "Other repos" section, kept up to date by seatwise. It lists only this seat's
repos, one per line: `- name (what it is): path. Clone from <remote> if missing.`

1. Find the repo the task needs in "Other repos". Open files there by their path.
2. If its folder is missing and the line names a remote, clone it to exactly that path. Don't clone anything else.
3. Work only in the home repo and the listed repos. A folder that isn't listed may belong to another seat: don't
   read it; tell the user, who can add it with `"{{seat}}" link <seat> <path>`.
4. After the user adds or removes repos, refresh the list: `"{{seat}}" home sync <seat>`.

Shared skills go in the home repo's `skills/` folder (Codex: `.agents/skills/`). seatwise copies them into the
seat on every start.
