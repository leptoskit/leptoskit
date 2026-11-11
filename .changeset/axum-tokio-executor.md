---
"release": patch
---

# Server rendering can spawn tasks

`leptoskit::server::serve_listener_with`, and with it `serve_listener`, `serve_router`, and `serve_router_with`, now makes tokio the global `any_spawner` executor when it starts serving, unless the app already chose one. Before, a server-rendered page that used `leptoskit::query`, a leptos resource, or anything else that spawns a task panicked with "tried to spawn a Future with Executor::spawn() before a global executor was initialized", unless the app knew to call `Executor::init_tokio()` itself.
