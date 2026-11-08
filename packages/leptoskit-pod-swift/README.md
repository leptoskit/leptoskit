# leptoskit-pod-swift

Swift implementation of the LK Pod little-endian wire codec for leptoskit RPC.

- Content-Type: `application/vnd.leptoskit.rpc.v1+bin`
- Envelope magic: `LK01`
- Conformance: `crates/leptoskit_pod/conformance/vectors.json`

See [`docs/adding-a-pod-language.md`](../../docs/src/rpc/adding-a-pod-language.md) (repository book) and the Agent Store guide `adding-a-pod-language.md`.

## Requirements

- Swift 5.9+ (`swift` on PATH)

## Test

```bash
cd packages/leptoskit-pod-swift
swift test
```

Or from the repo root:

```bash
scripts/pod-conformance-swift.sh
```

## API

- `Writer` — append primitives, length-prefixed bytes/strings, struct headers, variants
- `Reader` — symmetric reads; `finish()` rejects trailing bytes
- `wrap` / `unwrap` — `LK01` transport envelope
- `PodError` — typed errors matching the Rust codec
