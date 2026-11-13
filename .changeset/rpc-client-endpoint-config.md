---
"leptoskit_rpc": minor
"leptoskit_rpc_macros": patch
"leptoskit_rpc_codegen": minor
"leptoskit_cli": patch
---

Configured RPC endpoints now work for clients, not only for the server. The server has served the operations at `[rpc].path` and the live socket at `[rpc].live_path` since the last release, but the clients still called the compile-time defaults: the app's own client posted every call to `/rpc/<op>`, the browser's server-sent events fallback opened `/live/events` and posted frames to `/live/frames`, and the exported spec described every operation at `/rpc/<op>` however the app was configured.

The spec export now carries both mounts (`rpc_path` and `live_path`, spec version 4), composes every rpc operation's route from the configured prefix — a service's spec under the service's own prefix — and the foreign and generated SDK clients call those routes and open that socket, so `leptoskit rpc generate` output follows the configuration. The app's own client follows it too: `leptoskit dev` and `leptoskit build` compile the browser and native clients with `LEPTOSKIT_RPC_PATH` and `LEPTOSKIT_RPC_LIVE_PATH` from `leptoskit.toml`, and `ClientConfig::rpc_path`/`live_path` name the mounts by hand for builds that don't go through the CLI. A service's operations keep the default mounts under the service's prefix: `[rpc].path` moves the app's own. `scopes_for` matches rpc operations at the prefix this process serves, so a third-party token's scope requirements follow the operations when the prefix moves. Defaults are unchanged (`/rpc`, `/live`, legacy `/api`), as are auth headers, service credentials, and service origin isolation.
