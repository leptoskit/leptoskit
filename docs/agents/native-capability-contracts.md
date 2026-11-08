# Native capability contracts

A06's foundation: one versioned, typed model for what a generated native host can do, what an app requires of it, and what a code push release was allowed to assume.

## Why the string contract was not enough

`native.update_contract` is an app-owned compatibility string. It names a store build, and the signed `runtime.json` carries `{abi, shell, plugins}`. That catches a different ABI or shell and a missing plugin name, but it cannot say:

- which plugin flavour the host resolved (`rust.sync`, `rust.async`, `native.apple`, `service`), or whether the implementation was experimental;
- whether a plugin was merely granted, required, or optional;
- what non-secret plugin options changed;
- which destination flavour actually produced the build;
- whether a release was verified against a versioned contract or only passed the old string checks.

The old `runtime.json` also had no explicit format version. A host could not tell a legacy descriptor from one that silently omitted new fields.

## The model

`leptoskit_codepush::capability` owns the types:

- `HostCapabilityDescriptor` — the locally authoritative host contract: `format_version`, `platform`, `flavor`, `experimental`, `protocol` (`guest_abi`, `bridge`), sorted `capabilities`, and sorted `plugins`.
- `CapabilityEntry` — a stable identifier and one of `available`, `unsupported`, `experimental`, or `unknown`, plus a plain-language detail.
- `PluginCapability` — the plugin's resolved state and implementation flavour, its option key names, and the SHA-256 of the canonical non-secret option table. Values are never written.
- `RuntimeRequirements` — the signed guest's `required` and `optional` identifiers.
- `ContractCompatibility` — `verified`, `incomplete`, or `legacy`.

The CLI builds the descriptor in `crates/leptoskit_cli/src/native/registry/capability.rs` from the registry that actually resolved: `HostCapabilities::of(target)`, each selected `Plugin::implementation`, the missing-plugin reasons, and the app's option tables. It never re-derives support from a hand-written table.

Host-level identifiers:

| Identifier                                         | Meaning                                    |
| -------------------------------------------------- | ------------------------------------------ |
| `host.plugin.rust.sync`                            | Synchronous Rust plugins.                  |
| `host.plugin.rust.worker`                          | Worker or main-thread Rust plugins.        |
| `host.plugin.rust.async`                           | Async-host plugins.                        |
| `host.plugin.service`                              | App-owned Rust services.                   |
| `host.plugin.native.apple` / `.android` / `.arkts` | Native plugin dispatchers.                 |
| `host.update.codepush`                             | Automatic update checks.                   |
| `host.timer`                                       | The reserved `leptoskit.timer` plugin.     |
| `host.platform.experimental`                       | Present only on experimental destinations. |
| `plugin.<name>`                                    | A granted runtime plugin.                  |

Experimental Rust registrations are `experimental` only when `native.experimental-rust-plugins = true` selected them. Without the opt-in the plugin is `unsupported` with a detail naming the setting. Plugin/platform tables still choose the implementation first, so a native fallback is not called experimental just because a disabled Rust registration exists.

### Determinism

`HostCapabilityDescriptor::canonical_bytes` sorts capabilities and plugins, sorts and deduplicates option keys, serializes to JSON, then re-encodes every object with sorted keys. `digest()` is the SHA-256 of those bytes. Tests shuffle entries and map insertion order and require the same digest. The descriptor contains no machine paths, timestamps, package ids, or option values.

## Required and optional declarations

`NativeConfig` adds four backward-compatible lists:

```toml
[native]
plugins = ["http", "haptics"]
required-plugins = ["http"]
optional-plugins = ["haptics"]
required-capabilities = ["host.plugin.rust.async"]
optional-capabilities = ["host.plugin.native.android"]
```

- The lists must be unique, disjoint, and name granted plugins.
- A required entry fails registry resolution before host generation or packaging, whatever `missing-plugins` says.
- An optional entry warns with the platform and reason and builds without it.
- A grant in neither list keeps the existing `missing-plugins` behavior, so old configurations are unchanged.
- A declaration never grants authority. `native.plugins`, the generated registrant, the host's permissions, and plugin option policies stay independently enforced.

## Build output and code push binding

`native bundle` (and therefore `native prepare`/`build`) writes:

- `runtime.json` inside the signed bundle: legacy `abi`/`shell`/`plugins` plus `format_version = 2` and signed `requirements` (never a host descriptor).
- `capabilities.json` next to the generated bundle: the host descriptor and its digest, for diagnostics and documentation. The standalone `native bundle --out bundle` command writes `bundle.capabilities.json` beside the output directory, so packaging that directory never includes host evidence. Watch preparation writes `.leptoskit/<platform>/capabilities.json` and enforces requirements before scaffolding.

The mobile host's store-shipped `codepush.json` embeds the same descriptor in `runtime.json`-style `host`. `RuntimeContract::validate_host` rejects a host configuration that mixes guest requirements into the host side, omits the descriptor, or speaks a different guest ABI. Its plugin grants must match the descriptor's plugin entries exactly. A host-level `plugin.*` entry cannot substitute for a resolved plugin entry and grant.

`RuntimeContract::verify_guest` is called by `validate_container` for the store baseline, every downloaded release before it is staged, and every stored release at launch:

- No `format_version`: explicit legacy decoding. The old ABI/shell/plugin checks still run, and `UpdateReport.contract` is `legacy`.
- `format_version = 2`: requires signed `requirements`; the downloaded guest may not carry `host`; the guest ABI and bridge protocol must match the host, and every required identifier is looked up in the installed host descriptor. `unsupported`, `experimental`-without-opt-in, and `unknown` reject the release before storage. Optional identifiers never gate; explicitly optional plugins are excluded from the guest's legacy mandatory plugin list.
- Any other version is rejected with `UnsupportedContractVersion`.

The legacy plugin list records access grants, which may include plugins omitted under `missing-plugins = "warn"`. After checking explicit requirements, version 2 also checks the availability of those retained grants. If any is `unsupported` or `unknown`, the update retains its existing fallback behavior but reports `incomplete`, never `verified`. Marking such a plugin explicitly required rejects the update; marking it optional declares that the app can run without it and removes it from the legacy list. `verified` proves the declared requirements and retained plugin availability against build metadata, not that a device executed the app successfully.

The guest's requirements are covered by the release signature because `runtime.json` is a bundle file and the signature covers the manifest hash. A downloaded guest cannot supply the host side, so its labels are ignored.

The C update report, Swift `NativeSyncReport`, and Kotlin `NativeSync` preserve the contract state. Generated iOS and Android update controls display its meaning alongside the downloaded/current status. Older libraries that omit `contract` show compatibility as unreported; unknown state strings fail decoding rather than being promoted to verified. Device execution of these controls remains a separate acceptance gate.

## Evidence states

These are deliberately separate:

1. **Declared build support** — the descriptor. Derived from registry resolution; says what this build expects to provide.
2. **Runtime observation** — `platform_probe`, widget suites, code push journeys, and device validation. These can fail even when the descriptor says `available`.
3. **Store distribution trust** — the host descriptor travels in the store build like the signing keys and update policy; it is not a downloaded artifact.

The `host.update.codepush` capability is `available` only for iOS, iOS Simulator, Android, and Android x86-64 today, matching the generated hosts that actually call the update API. macOS, Windows, Linux, HarmonyOS, and the watch stubs report it `unsupported`.

`host.timer` follows the generated host that actually calls `lk_app_enable_timers` (or the backend's `enable_timers` in the desktop and terminal runners): macOS, iOS, iOS Simulator, Android, Android x86-64, Windows, Linux, the terminal, and HarmonyOS are `available`; the Wear OS and watchOS stubs are `unsupported` with a detail saying they run no guest. Requiring `host.timer` therefore fails before packaging on those stubs and verifies against older store builds only when their descriptor names it. The reserved timer is still not a `native.plugins` grant, and a descriptor entry says the generated host enables it, never that a device observed a sleep complete. The platform matrix in `docs/agents/platform-capability-matrix.md` is generated by `registry::capability::documentation_table()` and checked by a unit test.

## Legacy and compatibility policy

- `native.update_contract` and the legacy `abi`/`shell`/`plugins` fields keep their meaning. Existing projects and signed releases continue to work.
- A legacy release is never upgraded to `verified`; it is `legacy` in the update report.
- A versioned release whose unclassified plugin grants lack implementation evidence is `incomplete`. Required capability failures still reject the release; this state preserves legacy warning behavior without claiming complete compatibility.
- A versioned release cannot ship to a host that predates `format_version = 2`: that host's `deny_unknown_fields` decoding rejects it. This is the explicit compatibility boundary rather than silently ignoring unsigned extensions.
- A version 2 descriptor with missing fields, unknown versions, duplicate capabilities/plugins, unsorted option keys, or invalid option digests is rejected.

## Remaining gaps for the next wave

This foundation does not close all of A06. Concrete gaps, all visible in the resolved model:

- Windows and the terminal runner still register no runtime plugins; their descriptors correctly report every plugin unsupported. The Linux runner now hosts synchronous, worker, and async Rust plugins, but not app-owned services.
- HarmonyOS runs synchronous Rust and ArkTS plugins only: no worker, async, service, or code push.
- Wear OS and watchOS are experimental stubs with no plugin registration or code push.
- macOS has no automatic update checks, so `host.update.codepush` is unsupported there.
- `Plugin::implementation` does not yet expose per-method capability metadata; a plugin is one all-or-nothing entry.
- Runtime device evidence is not attached to the descriptor. A future evidence document or CI artifact should record probe results separately, never as fields a guest can claim.
- `host.timer` describes the generated host's timer wiring, not every app-owned runner or device lifecycle. Sleep delivery on a suspended or backgrounded device remains separate runtime evidence.

## Tests

Unit tests live in `leptoskit_codepush::capability` (determinism, versions, duplicates, required vs optional verification, option digest) and `leptoskit_cli::native::registry::capability_tests` (host flavour differences, experimental opt-in, required build failure, optional warnings, option/grant digest changes, timer availability, no values or paths, documentation matrix). `crates/leptoskit_codepush/tests/installed.rs` covers staging rejection before a release is stored, legacy reporting, guest-supplied host descriptors, unknown versions, and host ABI/version mismatches. `crates/leptoskit_bridge_ffi/tests/updates.rs` carries `verified` and `incomplete` through the C update report.

## Acceptance evidence, 7 November 2026

Final checks used the repository-pinned `nightly-2026-10-21` (`rustc 1.101.0-nightly (19833af72 2026-10-20)`) in an isolated rustup home with its `wasm32-unknown-unknown` std; the moving shared toolchain was not used.

Tested tree: the four A06 commits from `5730b1c0e30c63da3740ff2595ee093f8977e8d2` rebased onto `origin/main` at `794f7adb` (#665, including merged #609 timers, #615 stores, #619 plugin hardening, #621 rollback safety, #628 badge/shortcut support, #662 generated Linux runtime plugins, and #665 Linux examples), plus the acceptance fixes in the same pull request. The shared audit state and its full logs disappeared during the run; the deleting actor was not established; this table is the retained record, and the final checks were rerun with an isolated copy of the repository-pinned `nightly-2026-10-21` toolchain whose host files match the recovered read-only Nix store.

| Check                                                                   | Result                                                                                                      |
| ----------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `cargo nextest run -p leptoskit_codepush`                               | 175 passed                                                                                                  |
| `cargo nextest run -p leptoskit_config --all-features`                  | 93 passed                                                                                                   |
| `cargo nextest run -p leptoskit_cli`                                    | 1115 passed in the workspace run below                                                                      |
| `cargo nextest run -p leptoskit_bridge_ffi --features app`              | 86 passed, 1 skipped                                                                                        |
| `swift test` in `crates/leptoskit_bridge_ffi` against the built C ABI   | 16 passed                                                                                                   |
| Exact-source `NativeSyncReport` Foundation probe                        | 9 decode/message cases passed                                                                               |
| `devenv shell schema:update`                                            | regenerated `docs/src/schema.json` and both published schemas                                               |
| `devenv shell test:cargo`                                               | final run: 5789 workspace passed, 58 skipped; file routes 16; DevTools 43                                   |
| `devenv shell lint:clippy`, `lint:clippy:wasm` for `leptoskit_codepush` | passed with warnings denied                                                                                 |
| `devenv shell lint:push`                                                | passed: gitleaks, clippy, monostyle, leptoskit, docs, format, kotlin, tests, zizmor, releases, schema, deny |

Actual CLI cases against a generated project whose platform host depends on a plugin with runtime metadata:

- `lk native bundle --target tui` with `required-plugins = ["clipboard"]` failed before writing a bundle or descriptor; the diagnostic names `required-plugins` and `plugin.clipboard`.
- The same target with the plugin optional warned, wrote a descriptor with `plugin.clipboard` `unsupported`, and `lk native prepare tui` built the terminal runner.
- An unclassified TUI grant kept the grant in the signed guest runtime and reported it `unsupported` in the host descriptor; the new integration test verifies that pair as `incomplete`, never `verified`.
- `lk native bundle --target ios` with `required-plugins` succeeded and recorded `plugin.clipboard` as `native.apple`; `--target android` recorded `native.android`.
- `lk native prepare macos` and `lk native build macos` produced `App.app`, compiling the generated Swift mobile sources including the update controls.
- `lk native prepare ios-simulator` compiled the guest bundle and the iOS-simulator Rust static library.

Honest gaps from that run:

- The Android host build was not run: this machine has no Android SDK/NDK. Only the Android guest bundle and descriptor were produced.
- `lk native build ios-simulator` could not run: `xcodebuild` failed to load `IDESimulatorFoundation`/`CoreSimulator` on this host. That is an environment failure, not a code pass; no simulator or device UI claim is made.
- Linux, Windows, HarmonyOS, Wear OS, and watchOS generated host builds were not run on macOS. Linux's new synchronous, worker, and async Rust plugin support comes from #662's generated `PluginHost` wiring, not from a Linux run here; Windows/HarmonyOS/watch descriptors come from the same tested registry model and unit tests, and the watch stubs have no guest.
- No device or simulator executed a timer sleep. `host.timer` describes the generated host's wiring only.
- Kotlin was linted, not compiled: no Android toolchain. Swift production sources were compiled by the macOS host build, and the C ABI ran under `swift test`.
- The broader A06 gaps in the previous section remain open.

## Parent review, 8 November 2026

The parent review removed the directory-wide Gitleaks exception for dated audit directories. `.gitleaksignore` now lists only 177 exact, commit-specific historical findings: existing Git object IDs, independently matched source SHA-256 digests, and reviewed acceptance prose. The complete history scan passes with this list. Synthetic credential fixtures added to an existing audit checkpoint and a new audit directory are both detected; the previous directory exception missed both. New findings in these paths remain subject to the normal secret scan.

The twelve Python files reformatted during gate repairs have identical parsed syntax trees before and after this pull request. This comparison establishes that those formatting edits do not change their Python statements.
