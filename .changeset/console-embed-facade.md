---
"release": minor
---

# Embed the deployment service in your own app

An application with its own sign-in can host the deployment service in-process instead of handing people owner keys. `leptoskit_deploy_console`'s `Service` gains a typed facade — `overview`, `enqueue`, `operation`, `fleet`, `issue_token`, `revoke_token` — that the HTTP handlers now call too, so scope, revision and idempotency rules have one implementation. `Principal` is public with `owner()`, `scoped(scope, project)` and `with_actor`, and the recorded actor appears on operations and in history through a paired migration.

`Service::subscribe(operation_id)` streams progress events and the terminal operation, `GET /api/v1/operations/{id}/events` serves the same updates as server-sent events (the dashboard follows the stream and falls back to polling), and `Service::api_router()` nests under another axum app's path prefix without the dashboard page or script. The deployment engine's `CommandRunner` is re-exported from `leptoskit_cli::console`, so an embedded service runs the same worker given the `lk` executable path and data directory.
