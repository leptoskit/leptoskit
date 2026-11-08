# Adding a language to `leptoskit_pod`

Use this when implementing LK Pod for a language that does not yet have a runtime under `packages/leptoskit-pod-*`.

## Agent prompt (copy-paste)

```text
You are implementing LK Pod support for <LANGUAGE>.

Read and obey:
1. crates/leptoskit_pod/readme.md
2. docs/src/rpc/lk-pod.md
3. docs/src/rpc/migrations.md
4. crates/leptoskit_pod/conformance/vectors.json
5. An existing runtime twin: packages/leptoskit-pod-ts/

Deliverables:
- packages/leptoskit-pod-<lang>/ with encode/decode for the closed type grammar
- No dependency on protobuf / flatbuffers / rkyv
- Byte-identical encode of every vector in vectors.json; decode must reject the
  same malformed cases Rust rejects
- Unit tests + a harness invoked by scripts/pod-conformance.sh
- README: install, Content-Type, link to this page
- Wire codegen in crates/leptoskit_rpc_codegen MUST import this package

Closed type grammar:
  struct (disc:u16 + version:u16 + dense fields)
  enum (variant_disc:u16 + payload)
  Option/Result (u8 tag), Vec (u32 count), String/bytes (u32 len),
  bool/u8..u64/i8..i64 little-endian, nested structs/enums

Transport envelope: magic LK01 | codec_ver:u16 | msg_type:u16 | flags:u16 |
body_len:u32 | body. Content-Type: application/vnd.leptoskit.rpc.v1+bin

Migrations: servers climb inbound struct versions to current; clients encode
current. Live/events do not climb. Reject UnknownDiscriminator / UnknownVersion.

Done when: conformance script green; generated client smoke test round-trips
against the Rust fixture server.
```

## Checklist

| Step             | Done when                                                          |
| ---------------- | ------------------------------------------------------------------ |
| Package scaffold | Manifest + license + readme under `packages/leptoskit-pod-<lang>/` |
| Reader/Writer    | Primitives match the Rust API                                      |
| Struct/enum      | `disc:u16` + `version:u16` envelope; enum variant disc             |
| Envelope         | `LK01` framing                                                     |
| Conformance      | All vectors encode/decode; malformed set rejected                  |
| Codegen hook     | `leptoskit_rpc_codegen` emits imports of the package               |
| CI               | `scripts/pod-conformance.sh` lists the language                    |
| Docs             | Language row added to `lk-pod.md`                                  |
