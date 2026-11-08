# leptoskit_pod

LK Pod is leptoskit’s portable little-endian TLV wire format for RPC and live frames. It is **not** Rust’s in-memory layout: every generated client language speaks the same bytes.

- Content-Type: `application/vnd.leptoskit.rpc.v1+bin`
- Envelope magic: `LK01`
- Structs carry `[discriminator:u16][version:u16][payload…]`; enums carry a `u16` variant discriminator
- `Vec` / `String` use unbounded `u32` length prefixes
- Inbound RPC / jobs **climb** adjacent version transitions to current; responses encode current; live events do not migrate

See `docs/src/rpc/lk-pod.md`, `docs/src/rpc/migrations.md`, and `docs/src/rpc/adding-a-pod-language.md`. Conformance vectors live in `conformance/vectors.json`.

## Verification

| Check               | Command                                                                                                                                   |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Tests + conformance | `./scripts/pod-conformance.sh`                                                                                                            |
| 100% line coverage  | `devenv shell coverage:pod`                                                                                                               |
| Kani proofs         | `./scripts/pod-kani.sh` or `cargo kani -p leptoskit_pod` (requires [cargo-kani](https://model-checking.github.io/kani/install-kani.html)) |
| Mutation testing    | `./scripts/pod-mutants.sh` (requires `cargo install cargo-mutants`)                                                                       |
| Agent eval prompts  | `crates/leptoskit_pod/evals/readme.md`, `scripts/pod-eval-prompts.md`                                                                     |

Kani proof harnesses live in `src/proofs.rs` under `#[cfg(kani)]` and are ignored by normal `cargo build`.
