# Meta Horizon (platform extension)

<Callout kind="warning" title="Experimental">

The Meta Horizon platform extension (`horizon`) is experimental. Its TypeScript Custom UI scripts and offline `.zip` bundler are tested locally, and have not been run inside the Meta Horizon Worlds Desktop Editor or submitted to a live Devpost competition yet.

</Callout>

Meta's Horizon ecosystem has two separate targets:

- **Meta Quest headsets (Horizon OS)** run Android APKs with OpenXR and publish to the Meta Horizon Store. For headsets, use the first-party [Meta Quest](quest.md) destination (`--platform quest` and `leptoskit publish quest`).
- **Meta Horizon Worlds and Meta Horizon Creator Program (MHCP) hackathons** use either `horizon/ui` TypeScript scripts inside the Horizon Worlds Desktop Editor or self-contained offline HTML5/WebGL `.zip` bundles submitted on Devpost.

The `horizon` platform extension (`leptoskit_platform_horizon`, registered under `leptoskit native platform`) covers both of those creator workflows without adding a new variant to `app.platforms`.

## Scaffold the Meta Horizon host

```bash
leptoskit native platform list
leptoskit native platform capabilities horizon
leptoskit native platform scaffold horizon
```

Scaffolding writes `hosts/horizon/` with:

- `scripts/scene.ts`: applies leptoskit `version: 1` render transactions and maps Yoga layout frames and props to `horizon/ui` styles.
- `scripts/LeptosKitHost.ts`: a `horizon/ui` `UIComponent` that draws `View`, `Text`, `Pressable`, `ScrollView`, and `Image` nodes and emits `LeptosKitPressEvent` on clicks.
- `mhcp/index.html` and `mhcp/BUILD_LOG.md`: a portrait-oriented offline shell and build log template for MHCP Devpost submissions.
- `guest.rs` and `readme.md`: host constants and integration notes.

## Package an offline MHCP hackathon `.zip`

MHCP web-prototype competitions expect a self-contained `.zip` with `index.html` at the archive root, every script and asset bundled under relative paths, no external CDN or network requests, and a 35 MiB size limit.

Build your leptoskit web app in release mode, rewrite root-relative paths in `index.html`, validate every offline rule, and write a deterministic `.zip`:

```bash
leptoskit native platform package horizon
```

To package an existing build directory (such as `dist/` or `hosts/horizon/mhcp/`) without rebuilding:

```bash
leptoskit native platform package horizon --source dist --skip-build
```

Pass `--out <file.zip>` to choose the archive path (defaults to `.leptoskit/horizon/<name>-<version>-mhcp.zip`), and `--max-mb <n>` to adjust the size cap (defaults to `35`).

Before writing the archive, the packager checks:

- `index.html` is at the top level of the archive;
- precompressed server sidecars (`.br`, `.gz`) are omitted so they do not bloat the archive;
- root-relative asset references (`/pkg/...`, `/assets/...`) in HTML are rewritten to relative `./...` paths;
- no external `http://`, `https://`, or protocol-relative `//` URLs remain in HTML, CSS, or JavaScript;
- every relative asset referenced by `index.html` exists inside the archive;
- the final `.zip` fits inside `--max-mb`.

## Use the `horizon/ui` Custom UI scripts in Horizon Worlds

1. Open your world in the Meta Horizon Worlds Desktop Editor.
2. Copy `hosts/horizon/scripts/scene.ts` and `hosts/horizon/scripts/LeptosKitHost.ts` into the world's `scripts/` folder.
3. Add a Custom UI Gizmo to the scene and attach `LeptosKitHost`.
4. Send serialized `version: 1` render transactions through `LeptosKitTransactionEvent` (or `initialTransaction`) and subscribe to `LeptosKitPressEvent` for user clicks.

## What works and what doesn't

| Works | Doesn't work yet |
| --- | --- |
| `leptoskit native platform list` and `capabilities horizon` | Running an on-device Wasm interpreter inside Horizon Worlds (Horizon Worlds executes TypeScript only) |
| `leptoskit native platform scaffold horizon` (`horizon/ui` scripts + MHCP templates) | `TextInput`, `PasswordInput`, `Vector`, and `Rive` in `horizon/ui` Custom UI |
| `leptoskit native platform package horizon` deterministic offline `.zip` and validation | Programmatic `scroll_offset` on `horizon/ui` `ScrollView` and remote `Image` URLs |
| Rewriting leptoskit web `index.html` root paths to relative offline paths | Automated upload to Devpost or Horizon Worlds (upload the `.zip` on Devpost or copy scripts into Desktop Editor) |
| Meta Quest APK builds and Horizon Store uploads (see [Meta Quest](quest.md)) | Code push and runtime plugins on the `horizon` extension |
