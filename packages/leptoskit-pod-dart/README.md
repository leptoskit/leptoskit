# leptoskit-pod-dart

Dart implementation of the LK Pod wire codec for leptoskit RPC.

- Content-Type: `application/vnd.leptoskit.rpc.v1+bin`
- Envelope magic: `LK01`
- Conformance: `crates/leptoskit_pod/conformance/vectors.json`

## Requirements

- Dart SDK 3.3+ (`dart` on PATH)

## Test

From the repository root:

```bash
cd packages/leptoskit-pod-dart
dart test
```

Or:

```bash
scripts/pod-conformance-dart.sh
```

## API

Library `leptoskit_pod`: `Writer`, `Reader`, `wrap`, `unwrap`, `PodError`, `MessageType`.
