# Meta Horizon host (experimental)

This folder was scaffolded by `leptoskit native platform scaffold horizon` for ****APP_NAME**** (`__PACKAGE_ID__`).

## What is in this folder

| Path                       | Purpose                                                                                                              |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `scripts/scene.ts`         | Applies leptoskit `version: 1` render transactions and converts layout/props to `horizon/ui` styles                  |
| `scripts/LeptosKitHost.ts` | Meta Horizon Worlds `UIComponent` (`horizon/core` + `horizon/ui`) that renders the scene tree and emits press events |
| `scripts/scene_test.ts`    | Unit tests for the scene graph and `horizon/ui` style mapping (run with `node --test` or `deno test`)                |
| `mhcp/index.html`          | Portrait-oriented, offline-first HTML5 shell template for MHCP hackathon submissions                                 |
| `mhcp/BUILD_LOG.md`        | Build log template for MHCP Devpost submissions                                                                      |
| `guest.rs`                 | Guest platform constants and integration notes                                                                       |

## 1. Packaging an MHCP hackathon `.zip` submission

MHCP web-prototype competitions require a self-contained `.zip` with `index.html` at the archive root, all dependencies and assets bundled under relative paths, no external network/CDN requests, and a maximum size of 35 MiB.

Package your leptoskit web build (or any static build folder) and check every rule before submitting to Devpost:

```bash
leptoskit native platform package horizon
```

To package an existing directory (such as `dist/` or `hosts/horizon/mhcp/`) without rebuilding:

```bash
leptoskit native platform package horizon --source dist --skip-build
```

The deterministic archive is written to `.leptoskit/horizon/__APP_NAME__-__VERSION__-mhcp.zip`.

## 2. Using the `horizon/ui` Custom UI host in Meta Horizon Worlds Desktop Editor

1. Open your world in the Meta Horizon Worlds Desktop Editor.
2. Copy `scripts/scene.ts` and `scripts/LeptosKitHost.ts` into the world's `scripts/` folder.
3. Add a Custom UI Gizmo to the scene and attach `LeptosKitHost`.
4. Feed leptoskit `version: 1` render transactions into `LeptosKitHost.applyTransactionJson(json)` and listen for emitted `LeptosKitPressEvent` events (`nodeId`).

## Supported `horizon/ui` widgets

- `view`, `text`, `button`, `pressable`, `scroll_view`, `image`
- `text_input`, `password_input`, `vector`, and `rive` are not supported in `horizon/ui` Custom UI and raise an error if created.
