---
"leptoskit_pod": minor
"leptoskit_pod_macros": minor
"leptoskit_rpc": minor
"leptoskit_rpc_macros": minor
"leptoskit_rpc_codegen": minor
"leptoskit_config": minor
---

# LK Pod wire format

Portable little-endian TLV codec for RPC (`application/vnd.leptoskit.rpc.v1+bin`). Every data struct carries an implicit `u16` layout version; inbound RPC/jobs climb adjacent migrations to current while responses encode current. Language runtimes live under `packages/leptoskit-pod-*`, with optional JSON transport on the same `/rpc` routes.
