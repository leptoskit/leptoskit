---
"leptoskit_cli": patch
---

# Reading a watched file does not restart the app

`leptoskit dev` ignores filesystem access events before scheduling a rebuild or marking the configuration, database tables, or Git branch as changed. On Linux, the compiler and CLI reading their inputs could otherwise trigger another build repeatedly, even without an edit. Source edits and file replacements still rebuild the app.
