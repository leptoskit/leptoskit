---
leptoskit_rpc: patch
leptoskit_rpc_codegen: patch
---

Record server argument defaults in exported schemas without executing their default functions. Strict Go, Rust and CLI generation now reports nullable arguments whose omission invokes a server default but whose explicit null cannot be represented by the generated client. Ordinary optional arguments remain accepted.

Strict generation also rejects query/form parameters containing free-form JSON, whose empty arrays and objects cannot round-trip through that codec. Use JSON input encoding for those endpoints. Typed lists and maps remain supported.
