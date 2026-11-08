# leptoskit-pod-kotlin

Kotlin (JVM) implementation of the LK Pod wire codec for leptoskit RPC.

- Content-Type: `application/vnd.leptoskit.rpc.v1+bin`
- Envelope magic: `LK01`
- Conformance: `crates/leptoskit_pod/conformance/vectors.json`

## Requirements

- JDK 17+ (toolchain 21 in `build.gradle.kts` when available)
- Gradle 8+ (or use the Gradle wrapper once generated)

## Test

From the repository root (so tests find `vectors.json`):

```bash
cd packages/leptoskit-pod-kotlin
gradle test
```

Or:

```bash
scripts/pod-conformance-kotlin.sh
```

## API

Package `dev.leptoskit.pod`: `Writer`, `Reader`, `wrap`, `unwrap`, `PodError`, `MessageType`.
