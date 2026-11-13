---
leptoskit_cli: patch
---

# Generate auth in the selected backend package

Auth generation follows `build.server_package`, including a named target's resolved backend, while keeping client selection independent. It defaults to `app.name` when no server package is configured. The generator no longer writes server modules or dependency features into an unselected package that happens to share the application's name.
