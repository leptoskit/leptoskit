---
"leptoskit_rpc": minor
"leptoskit_config": minor
"leptoskit_cli": patch
---

# `[rpc].path` and `[rpc].live_path` decide where routes serve

`RpcConfig.path`, `live_path`, and `transports` were accepted and validated but no server read them: every app served `/rpc` and `/live` whatever the config said. Now they decide the mounts, once, at the app's entrypoint.

The generated app reads `[rpc]` from `leptoskit.toml` and applies it: operation routes serve at `path` (with the JSON routes kept on the legacy `/api` prefix while JSON is a transport), and `Server::rpc_config` carries the resolved live mount through `Server::live`. `Live` takes the same config, so a router built by hand mounts the socket at `live_path` — `/live` while the config stays at its defaults — with its server-sent events and frame posts right next to it. `path` without `live_path` moves the socket to `{path}/live`; an explicit `live_path` mounts it anywhere.

`leptoskit rpc spec` exports the configured live mount, so regenerated clients open their socket where the app serves it. Generated HTTP calls still name the compile-time `/rpc/<op>` route: keep the default `path` when generated clients call the app directly, and use a custom one behind a proxy or gateway that remounts the prefix.
