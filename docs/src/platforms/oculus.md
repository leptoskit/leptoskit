# Meta Quest / Oculus

<Callout kind="warning" title="Experimental">

Meta Quest support is early. The platform extension crate and host stubs exist; OpenXR packaging, store install, and a live compositor loop do not. Use it to experiment with Quest-shaped hosts, not to ship.

</Callout>

Quest is an **unsupported-class** destination delivered as a [platform extension](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_platform_oculus) rather than a full first-party widget backend. Templates and the capability manifest live in `leptoskit_platform_oculus`. The CLI registers that crate next to HarmonyOS so niche OS shells can grow without enlarging the core forever.

Day-one render strategy is **foreign-headless**: a Wasm guest records mutations; a future Quest shell applies render transactions to a 2D panel (and later world-space quads).

## What works and what doesn't

| Works                                                         | Doesn't work yet                     |
| ------------------------------------------------------------- | ------------------------------------ |
| `leptoskit new --platform oculus` and `native prepare oculus` | OpenXR / Meta Spatial packaging      |
| Host stubs under `hosts/oculus/`                              | Installing on a headset              |
| Capability manifest in `leptoskit_platform_oculus`            | Code push, DevTools, runtime plugins |
| Destination id `oculus` in `app.platforms`                    | Pixel-perfect text measurement       |

## Create a project

```sh
leptoskit new arena --platform oculus
cd arena
leptoskit native prepare oculus
```

The starter depends on `leptoskit_native` and `leptoskit_platform_oculus`. Kotlin stubs live under `hosts/oculus/android/`. Guest notes live in `hosts/oculus/guest.rs`.

## Build

```sh
leptoskit native build oculus
```

Packaging returns an experimental error until an OpenXR host lands. Prefer `prepare` while iterating on stubs.
