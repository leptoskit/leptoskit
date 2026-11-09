---
"release": patch
---

# Preserve declared service listen variables

Allow the graph process adapter to use an explicitly declared listen-address environment variable, including the playground's `PLAY_BIND`. Require its value to match the private loopback address instead of restricting services to toolkit-specific variable names. Executable graph activation and real-host acceptance remain pending.
