---
"release": patch
---

Honor the selected application's frontend lint rules, exclusions and Clippy setting when linting. Lint's partial configuration loader now follows `--app-target` and `app.default_target`, while still ignoring unrelated unfinished settings. Server-only targets keep the root lint configuration.
