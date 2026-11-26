---
name: {{name}}
description: Open a terminal running the CLI (Claude Code or Codex) for the current project on this seat, optionally with a session profile such as experimental settings. Use when the user wants to continue in a terminal, run the CLI next to the desktop app, or try experimental CLI features for the active project.
---

# Open a terminal for this project

The user works across the desktop app and the terminal. This opens the CLI for the same project, on the same seat
(account), so nothing crosses into another seat: in the app's own terminal panel when there is one, else in a new
terminal window.

1. Work out the project folder: the folder this session works in (`$CLAUDE_PROJECT_DIR`, else the current folder).
2. Where it opens:
   - **In the app's terminal panel**, when you have a tool that starts a command in a terminal tab inside the app
     this session runs in (the Claude desktop app's Terminal panel, beside the conversation). Get the command, from
     that folder:

     ```sh
     "{{seat}}" term --print
     ```

     and run the line it prints with that tool, in a new tab in the project folder (a full absolute path). Don't
     change the line.
   - **Otherwise**, a new terminal window. Run, from that folder:

     ```sh
     "{{seat}}" term
     ```

   Both take the same options:

   Only when the user asks for them:
   - `--with <profile>`: settings for this terminal session only, e.g. `--with experimental` (the config's
     `terminal.profile` is used when none is given). `"{{seat}}" profile <seat>` lists the profiles.
   - `--seat <name>`: another seat. Only if the user names it; never pick one yourself.
   - `--home`: start in the seat's home repo instead of the project.
   - `--dir <path>`: another folder of the same seat.
   - anything after `--` goes to the CLI, e.g. `-- --continue`.
3. Tell the user where the terminal opened and on which seat (`term` prints it; the printed line names it). The new session starts fresh:
   it doesn't see this conversation, so offer a short summary they can paste if the work continues there.

If the command refuses because the folder belongs to another seat, report that; don't work around it.

seatwise hides account identifiers in what it prints (`<org:work>`, `<email:work>`). Don't read its config file
or any seat's account or credential files to find them, and don't try to reveal them.
