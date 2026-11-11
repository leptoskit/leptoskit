---
"release": patch
---

# Find auth targets through Cargo

`lk generate auth` resolves client and backend packages through Cargo metadata. Nested app configurations, workspace globs, inherited dependencies, and custom library or binary source paths now receive modules beside the selected target. The server embeds the selected app configuration and resolves its paths relative to that app, even when its Cargo package lives elsewhere in the workspace.

Ambiguous binaries, missing dependencies, renamed toolkit imports, and source targets outside the workspace fail before generation writes files. An explicitly disabled backend is left alone. Existing application roots and database policies still need the integration steps printed by the command.

When the client and backend share a package, the command explains how to enable `leptoskit/server` for the backend build without adding server dependencies to browser or guest builds.
