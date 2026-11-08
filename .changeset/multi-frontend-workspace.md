---
"leptoskit_config": minor
"leptoskit_cli": minor
---

# Multiple frontend applications in one workspace

`leptoskit new --frontends landing,app,admin` scaffolds a `leptoskit.workspace.toml` with separate apps under `apps/` (landing and admin web-only; `app` keeps native hosts). Workspace applications may declare an optional `kind` (`landing`, `product`, `admin`, `server`) for tooling. Docs cover the multi-frontend layout; shared auth across frontends remains out of scope.
