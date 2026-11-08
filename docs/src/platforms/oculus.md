# Meta Quest / Oculus (platform extension)

Quest headsets that need a shippable Android-host APK today use the first-party [Meta Quest](quest.md) destination (`quest`, `[native.quest]`).

This page covers the **experimental platform extension** (`oculus`): host stubs and a capability manifest in `leptoskit_platform_oculus`, registered with `leptoskit native platform`. It is the foreign-headless OpenXR shell track from the platform-extensions design, not a second closed-enum destination.

## What works

| Ready                                                    | Not yet                          |
| -------------------------------------------------------- | -------------------------------- |
| `leptoskit native platform list` / `capabilities oculus` | OpenXR / Meta Spatial packaging  |
| `leptoskit native platform scaffold oculus`              | Installing via this extension id |
| Host stubs under `hosts/oculus/`                         | Code push, DevTools, plugins     |
| Capability manifest in `leptoskit_platform_oculus`       | Pixel-perfect text measurement   |

```bash
leptoskit native platform scaffold oculus
```

Kotlin stubs live under `hosts/oculus/android/`. Guest notes live in `hosts/oculus/guest.rs`.

For a headset install path that exists today, use [Meta Quest](quest.md) instead.
