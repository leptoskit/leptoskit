# LK Pod

LK Pod is a portable little-endian TLV codec that keeps Rust ADTs intact across TypeScript, Swift, Kotlin, Go, Dart, and Python. It is parked for evaluation and is not mounted on an RPC route. The server still speaks JSON.

- **Content-Type**, when a transport mounts it: `application/vnd.leptoskit.rpc.v1+bin`
- **Designed entrypoint:** `POST /rpc/<op>` (override with `[rpc].path`)
- **Designed selection:** `transports = ["pod", "json"]`, with Pod as the default when both are enabled

Authors mark types with `#[pod::data]` (or `#[rpc::data]` once re-exported). Every data **struct** carries an implicit layout **version** (little-endian `u16`). Enums use a `u16` variant discriminator. You never write field ids — [migrations](./migrations.md) own evolution.

See [Adding a Pod language](./adding-a-pod-language.md) for the conformance prompt used when bringing up a new runtime.
