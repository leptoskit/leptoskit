---
"release": patch
---

# Resume generated live clients from applied results

TypeScript, Python, Dart, Go, Rust, and CLI clients keep their applied cursor when a server welcomes them. A reconnect interrupted before its recovered results arrive now resumes from the previous complete transition, and pending mutations cannot use the welcome's server version as evidence that their results are on screen.
