# Pod migrations

Migrations let an **old client keep talking to a new server**.

Requests climb to the server’s current struct layout before the handler runs. Responses encode at the current layout — old clients may fail to decode the reply, and that is intentional. Live / event logs keep historical versions readable as-is (no climb).

## Wire identity

Every `#[pod::data]` / `#[rpc::data]` **struct** is:

```text
[version:u16][payload…]
```

`version` is the layout generation for that struct type (little-endian `u16`). It is bumped when the field layout changes. The surrounding op, job type, or parent field already names _which_ type the bytes are; enums address variants with their own `u16` discriminator.

Optional macro arg: `#[pod::data(version = 1)]` (default `0`).

## Where climb runs

| Surface                 | Policy                                                                           |
| ----------------------- | -------------------------------------------------------------------------------- |
| RPC requests / job args | Decode at on-wire version → adjacent `vN → vN+1` transitions → current → handler |
| RPC responses           | Always current version                                                           |
| Live / immutable logs   | Version-not-migrate; keep historical decoders                                    |

`leptoskit_pod::climb` rewrites a full struct buffer up to `current`. Gaps and unknown future versions fail closed.

## Author workflow

1. Edit the Rust data struct.
2. Drift against the frozen snapshot fails the build (or `lk rpc migrations create` detects it).
3. Create the next version and fill the adjacent transition.
4. Ship: old clients still POST; new clients encode the new version.

See [LK Pod](./lk-pod.md) for the codec overview.
