---
"release": patch
---

# Preserve declared service listen variables

Allow the graph process adapter to use an explicitly declared listen-address environment variable, including the playground's `PLAY_BIND`. Require its value to match the private loopback address instead of restricting services to toolkit-specific variable names. Executable release plans bind the declared address and account. Real-host service acceptance remains separate from static release acceptance.
