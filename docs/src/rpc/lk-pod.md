# LK Pod

LK Pod is the default wire format for leptoskit RPC: a portable little-endian TLV encoding that keeps Rust ADTs intact across TypeScript, Swift, Kotlin, Go, Dart, and Python.

- **Content-Type:** `application/vnd.leptoskit.rpc.v1+bin`
- **HTTP entrypoint:** `POST /rpc/<op>` (override with `[rpc].path`)
- **JSON fallback:** opt in with `transports = ["pod", "json"]`

Authors mark types with `#[pod::data]` (or `#[rpc::data]` once re-exported). Every data **struct** carries an implicit layout **version** (little-endian `u16`). Enums use a `u16` variant discriminator. You never write field ids — [migrations](./migrations.md) own evolution.

See [Adding a Pod language](./adding-a-pod-language.md) for the conformance prompt used when bringing up a new runtime.
