---
leptoskit_cli: patch
---

# Refuse graph-root auth generation before writes

Auth generation checks that it is running in an application configuration before writing client screens or dependency features. Deployment graph roots now fail before any generation writes, including with `--force`, instead of rejecting the application-only auth settings after partially changing a workspace client.
