---
"leptoskit_rpc": minor
"leptoskit_rpc_macros": minor
"leptoskit_config": minor
---

# JSON RPC wire and ambient context

Queries and mutations speak JSON through serde on `POST /rpc/<name>`. `[rpc]` sets `path`, `live_path`, `transports`, and `default_transport`; the only transport is `json`. A query or mutation can omit its context argument and call `rpc::cx()` or `rpc::mutation_cx()`.
