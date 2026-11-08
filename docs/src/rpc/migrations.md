# Pod migrations

Migrations let an **old client keep talking to a new server**.

Requests climb to the server’s current struct layout before the handler runs. Responses encode at the current layout — old clients may fail to decode the reply, and that is intentional. Live / event logs keep historical versions readable as-is (no climb).

## Wire identity

Every `#[pod::data]` / `#[rpc::data]` **struct** is:

```text
[discriminator:u16][version:u16][payload…]
```

| Field             | Default width | Role                                                                                                          |
| ----------------- | ------------- | ------------------------------------------------------------------------------------------------------------- |
| **discriminator** | `u16`         | Stable type id for this struct. Assigned when the type is first published; never reused for a different type. |
| **version**       | `u16`         | Layout generation for _this_ discriminator. Bumped when the field layout changes.                             |

Enums stay variant-addressed (`variant_disc:u16` + payload). A struct payload inside a variant carries its own `[disc][version][…]`.

Optional macro args: `#[pod::data(disc = 7, version = 1)]`. Both default to `0` until schema tooling assigns them.

## Where climb runs

| Surface                 | Policy                                                                           |
| ----------------------- | -------------------------------------------------------------------------------- |
| RPC requests / job args | Decode at on-wire version → adjacent `vN → vN+1` transitions → current → handler |
| RPC responses           | Always current version                                                           |
| Live / immutable logs   | Version-not-migrate; keep historical decoders                                    |

`leptoskit_pod::climb` rewrites a full struct buffer up to `current` for a known discriminator. Gaps and unknown future versions fail closed.

## Author workflow

1. Edit the Rust data struct.
2. Drift against the frozen snapshot fails the build (or `lk rpc migrations create` detects it).
3. Create the next version and fill the adjacent transition.
4. Ship: old clients still POST; new clients encode the new version.

See [LK Pod](./lk-pod.md) for the codec overview.
