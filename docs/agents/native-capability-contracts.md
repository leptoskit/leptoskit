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
- `ContractCompatibility` — `verified` or `legacy`.

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
- `capabilities.json` next to the bundle: the host descriptor and its digest, for diagnostics and documentation.

The mobile host's store-shipped `codepush.json` embeds the same descriptor in `runtime.json`-style `host`. `RuntimeContract::validate_host` rejects a host configuration that mixes guest requirements into the host side, omits the descriptor, or speaks a different guest ABI.

`RuntimeContract::verify_guest` is called by `validate_container` for the store baseline, every downloaded release before it is staged, and every stored release at launch:

- No `format_version`: explicit legacy decoding. The old ABI/shell/plugin checks still run, and `UpdateReport.contract` is `legacy`.
- `format_version = 2`: requires signed `requirements`; the downloaded guest may not carry `host`; every required identifier is looked up in the installed host descriptor. `unsupported`, `experimental`-without-opt-in, and `unknown` reject the release before storage. Optional identifiers never gate.
- Any other version is rejected with `UnsupportedContractVersion`.

The guest's requirements are covered by the release signature because `runtime.json` is a bundle file and the signature covers the manifest hash. A downloaded guest cannot supply the host side, so its labels are ignored.

## Evidence states

These are deliberately separate:

1. **Declared build support** — the descriptor. Derived from registry resolution; says what this build expects to provide.
2. **Runtime observation** — `platform_probe`, widget suites, code push journeys, and device validation. These can fail even when the descriptor says `available`.
3. **Store distribution trust** — the host descriptor travels in the store build like the signing keys and update policy; it is not a downloaded artifact.

The `host.update.codepush` capability is `available` only for iOS, iOS Simulator, Android, and Android x86-64 today, matching the generated hosts that actually call the update API. macOS, Windows, Linux, HarmonyOS, and the watch stubs report it `unsupported`. The platform matrix in `docs/agents/platform-capability-matrix.md` is generated by `registry::capability::documentation_table()` and checked by a unit test.

## Legacy and compatibility policy

- `native.update_contract` and the legacy `abi`/`shell`/`plugins` fields keep their meaning. Existing projects and signed releases continue to work.
- A legacy release is never upgraded to `verified`; it is `legacy` in the update report.
- A versioned release cannot ship to a host that predates `format_version = 2`: that host's `deny_unknown_fields` decoding rejects it. This is the explicit compatibility boundary rather than silently ignoring unsigned extensions.
- A version 2 descriptor with missing fields, unknown versions, duplicate capabilities/plugins, unsorted option keys, or invalid option digests is rejected.

## Remaining gaps for the next wave

This foundation does not close all of A06. Concrete gaps, all visible in the resolved model:

- Windows, Linux, and the terminal runner still register no runtime plugins; their descriptors correctly report every plugin unsupported.
- HarmonyOS runs synchronous Rust and ArkTS plugins only: no worker, async, service, or code push.
- Wear OS and watchOS are experimental stubs with no plugin registration or code push.
- macOS has no automatic update checks, so `host.update.codepush` is unsupported there.
- `Plugin::implementation` does not yet expose per-method capability metadata; a plugin is one all-or-nothing entry.
- Runtime device evidence is not attached to the descriptor. A future evidence document or CI artifact should record probe results separately, never as fields a guest can claim.
- The timer plugin branch (A07) is unmerged and is intentionally not advertised in this matrix or the descriptor until its generated hosts land.

## Tests

Unit tests live in `leptoskit_codepush::capability` (determinism, versions, duplicates, required vs optional verification, option digest) and `leptoskit_cli::native::registry::capability_tests` (host flavour differences, experimental opt-in, required build failure, optional warnings, option/grant digest changes, no values or paths, documentation matrix). `crates/leptoskit_codepush/tests/installed.rs` covers staging rejection before a release is stored, legacy reporting, guest-supplied host descriptors, unknown versions, and host ABI/version mismatches.
