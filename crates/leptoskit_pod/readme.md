# leptoskit_pod

LK Pod is leptoskit’s portable little-endian TLV wire format for RPC and live frames. It is **not** Rust’s in-memory layout: every generated client language speaks the same bytes.

- Content-Type: `application/vnd.leptoskit.rpc.v1+bin`
- Envelope magic: `LK01`
- Structs carry an implicit version byte; enums carry a `u16` discriminator
- `Vec` / `String` use unbounded `u32` length prefixes

See `docs/src/rpc/lk-pod.md` and `docs/src/rpc/adding-a-pod-language.md`. Conformance vectors live in `conformance/vectors.json`.
