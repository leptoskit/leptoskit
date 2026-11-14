# LeptosKit deployment service

<!-- {=deploy_consoleSummary} -->

One optional service hosts the dashboard, durable operation queue, scoped agent API and analytics history. It uses SQLite and reuses the existing `OpenTofu`/Kamal engine through fixed CLI operations. No Redis or separate monitoring database is required.

<!-- {/deploy_consoleSummary} -->

```sh
# On a trusted worker, with LEPTOSKIT_CONTROL_KEY and provider secrets configured:
lk console serve --project /srv/app --url https://deploy.example.com
# From the app checkout, with its management credential available:
lk console connect --url https://deploy.example.com
lk infra plan
lk infra apply
lk deploy
```

The worker needs Docker, OpenTofu, Kamal 2, Git and SSH. Configure TLS or a Cloudflare Tunnel in front of the default localhost listener. Registered repositories share one owner's trust and worker credentials; this is not a public multi-tenant executor. A normal Git-commit release uses a temporary checkout and an immutable image; prebuilt image tags are supported too.

The `server` feature enables `control::{Service, Store, Executor}`. The dashboard is a real LeptosKit builder tree, rendered on the server with a small browser interaction module. API and browser requests use the same operation contract, authorization, revision and idempotency checks. Browser sessions use HttpOnly cookies. Worker output remains in bounded private files; the API exposes typed status, progress events and reviewed resource actions.

## Embedding the service in another app

An app with its own sign-in can host the service in-process instead of giving people owner keys. The typed facade on `Service` enforces exactly what the HTTP API does, without a network hop:

```rust
use leptoskit_deploy_console::control::{Principal, Scope};

let admin = Principal::scoped(Scope::Deploy, "notes").with_actor("admin@example.com");
let operation = service.enqueue(&admin, "notes", &intent)?;
```

`Principal` is public: `Principal::owner()` acts with the owner's authority, and `Principal::scoped(scope, project)` covers one registered project. `overview`, `operation`, `fleet`, `issue_token` and `revoke_token` return typed errors, so the embedder's own UI can show them directly. `enqueue` records the principal's actor on the operation and shows it in history.

`Service::subscribe(operation_id)` streams journal updates — progress events, then the terminal operation — over a broadcast channel, and `GET /api/v1/operations/{id}/events` serves the same updates as server-sent events. The dashboard uses the stream and falls back to polling. `Service::api_router()` carries only the API routes, so another axum app can nest it under its own path prefix and keep serving its own pages; the dashboard page and script stay on `Service::router()`.

To run the same deployment engine from an embedded service, construct `leptoskit_cli::console::CommandRunner::new(data_dir, lk_binary, service)` and pass it to `service.worker(...)`. The CLI keeps one implementation of the runner; the executor still needs the registered repositories and provider credentials of a trusted worker machine.

The app analytics exporter sends bounded snapshots to a project-scoped ingest endpoint. The store upserts per-producer windows and aggregates reporting instances without counting retries twice. Its paired migrations, online backup, offline restore and schema rollback are exposed by `lk console`. Restored tokens are revoked. See the public deployment-service and analytics guides for setup, retention and recovery limits.

The original design study is still available through `model`, `store`, `view` and the `preview` example. It deliberately uses fixture data and does not issue cloud operations. Its target/availability previews are separate from the live single-owner service; Kubernetes and hosted multi-tenant execution remain separate adapters.

Validation runs through the repository's devenv shell:

```sh
cargo test -p leptoskit_deploy_console --all-features
cargo test -p leptoskit_cli --test infra
cargo check -p leptoskit_deploy_console --target wasm32-unknown-unknown --features csr
```
