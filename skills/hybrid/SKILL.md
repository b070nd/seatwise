---
name: {{name}}
description: Set up the command-line workflow for the current project next to the desktop app, on the same seat (account), then open a terminal there. Checks that the project belongs to this seat, installs the seat's skills, optionally sets a use-case preset (explore, build, review, quick) and session settings such as experimental features. Use when the user wants to start hybrid work, set up the CLI or terminal for this project, or work in the terminal alongside the desktop app.
---

# Set up the command line next to this session

The user works in the desktop app and wants the CLI for the same project on the same seat. `seat hybrid` does the
setup and opens the terminal. It refuses to cross into another seat's folders.

1. Check first; this changes nothing:

   ```sh
   "{{seat}}" hybrid --check
   ```

   It reports the seat, the project, whether the folder belongs to the seat, whether the skills are current,
   the project's preset, and what the terminal would run.

2. Ask the user only about what the check found open:
   - "owner not assigned": should this project belong to this seat? If yes, add `--link`.
   - "preset none": do they want a use-case preset for this project? `"{{seat}}" preset list` shows them
     (explore: reading and planning; build: day-to-day coding; review: careful review; quick: small fixes).
     If they pick one, add `--preset <name>`.
   - Experimental or other session-only settings: add `--with <profile>` (`"{{seat}}" profile <seat>` lists them).

3. Run it with the chosen options:

   ```sh
   "{{seat}}" hybrid [--link] [--preset <name>] [--with <profile>]
   ```

4. Pass on the report. The terminal session starts fresh and doesn't see this conversation: offer a short summary
   of the current task that the user can paste there.

Never pass `--seat` unless the user names a seat. If the command refuses (another seat's folder, a disabled seat),
report it and stop.

seatwise hides account identifiers in what it prints (`<org:work>`, `<email:work>`). Don't read its config file
or any seat's account or credential files to find them, and don't try to reveal them.
