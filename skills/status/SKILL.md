---
name: {{name}}
description: Show which seat (account) this session runs on, its plan, how much of its 5-hour and weekly limits is used, and the usage profile that fits. Use when the user asks which account they're on, how much usage is left, whether to switch to a lighter model, or why something is blocked by seatwise.
---

# Seat status

1. Which seat owns this folder: `"{{seat}}" owner`
2. All seats, their accounts, plans, identity checks, hooks and profiles: `"{{seat}}" list`
3. Usage left for a seat: `"{{seat}}" usage <seat>`. It asks Anthropic with that seat's own login; on macOS
   the keychain may ask the user to allow it. Never print, copy or store a token.
4. The profile that fits the plan (and this week's usage): `"{{seat}}" profile <seat> --usage`

Answer in a few lines: seat, account, plan, usage per window with reset time, and the suggested profile.
Applying a profile changes the seat's settings.json: run `"{{seat}}" profile <seat> <name>` only when the user
says so. If seatwise blocked something, `"{{seat}}" doctor` explains why; don't use `SEATWISE_FORCE` yourself (it is ignored inside a session, and the guard treats setting it as a bypass).

seatwise hides account identifiers in what it prints (`<org:work>`, `<email:work>`). Don't read its config file
or any seat's account or credential files to find them, and don't try to reveal them.
