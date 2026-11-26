---
name: {{name}}
description: Set up the command-line workflow for the current project next to the desktop app, on the same seat (account), then open a terminal there. Checks that the project belongs to this seat, installs the seat's skills, optionally sets a use-case preset (explore, build, review, quick) and session settings such as experimental features. Use when the user wants to start hybrid work, set up the CLI or terminal for this project, or work in the terminal alongside the desktop app.
---

# Set up the command line next to this session

The user works in the desktop app and wants the CLI for the same project on the same seat. `seat hybrid` does the
setup and opens the terminal. It refuses to cross into another seat's folders.

1. Get the plan; this changes nothing:

   ```sh
   "{{seat}}" hybrid --json
   ```

   It has the seat, the project, one entry per step in `steps` (`owner`, `skills`, `preset`, `session`, `terminal`,
   each `ok`, `change` or `open`), and in `choices` only what is still up to the user. Steps marked `change` (the
   seat's skills, for one) happen without asking.

2. Ask the user about each entry in `choices`, and nothing else. Offer its `options` with their `label`s; an option
   with `"experimental": true` turns on experimental Claude Code features (`--with experimental` is Code Mode), so
   say so. Each option's `flag` is what to add to the command; an option without a flag leaves that step as it is.
   The options are already checked: only settings Claude Code takes for that kind of session are offered. If
   `choices` is empty, go straight on.

3. Run it with the chosen flags:

   ```sh
   "{{seat}}" hybrid [--link] [--preset <name>] [--with <profile>]
   ```

   When you have a tool that starts a command in a terminal tab inside the app this session runs in (the Claude
   desktop app's Terminal panel, beside the conversation), add `--print`: the setup is the same, but instead of a new
   terminal window the report's `terminal` step holds the command. Run that line with the tool, in a new tab in the
   project folder (a full absolute path), unchanged.

4. Pass on the report. The terminal session starts fresh and doesn't see this conversation: offer a short summary
   of the current task that the user can paste there.

Never pass `--seat` unless the user names a seat. If the command refuses (another seat's folder, a disabled seat),
report it and stop.

seatwise hides account identifiers in what it prints (`<org:work>`, `<email:work>`). Don't read its config file
or any seat's account or credential files to find them, and don't try to reveal them.
