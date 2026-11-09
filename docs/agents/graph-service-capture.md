# Graph service capture evidence

Graph build receipts now use format 2. Static and SSR format-1 receipts remain readable. Format-1 service receipts have no resolved source/settings contract and are rejected; rebuilding is required. Executable release integration consumes this evidence; Rust and real host acceptance remain pending.

The service builder uses the existing selected-only resolver and independent package build. Before building it retains exact workspace Cargo.lock and manifest bytes (at most 256 files and 16 MiB per service), package name/version, merged settings, requested features, default-feature behavior, and dev/release profile. Workspace-inherited versions come from the captured workspace manifest. A path source is represented by a portable placeholder in settings; the input mapping contains project-relative paths, never machine-specific absolute paths.

The current supported envelope is workspace packages and project-contained path packages whose workspace manifests are available through Cargo metadata. Nonmember path dependencies, out-of-project sources, registry/git/remote sources, missing locks, and service secret references fail explicitly. Declared `env` settings are literal, non-secret public artifact inputs. In particular, registry versions and services.lock alone do not establish the dependency lock used by an external Cargo build. Supporting registry/git capture remains work to do: capture services.lock plus the fetched source workspace lock/manifests and bind the resolver's established source identity without persisting cache paths. There is no fallback that emits an executable contract without evidence.

Exact manifest capture permits the exact `[package.metadata.leptoskit.service.env]` table after literal-value validation. Other environment/credential keys and absolute paths outside that declared table are rejected. It does not redact a Cargo input and claim the modified bytes were used. Input documents must not contain secrets in comments or arbitrary metadata: this check is not a general secret detector.

Inputs are rechecked before/after build, copying and receipt publication. The private service tree has pinned membership, modes and byte digests. These digests and the typed descriptor are covered by the independently supplied manifest SHA-256. Verification re-merges declarations with the copied package manifest, checks package/version/profile and the original project input bytes, and executes neither Cargo nor the executable. Verification therefore requires the corresponding source input files in the project. Rewriting the bundle and its convenience checksum does not replace the original independent pin.

## Release consumer interface

`Checked::executable_inventory(&[String]) -> Result<Vec<Executable>>` validates the selection and rechecks inputs before and after returning. Each entry has `unit`, `backend: Member`, and `service: Option<ServiceEvidence>`. Service evidence includes resolved `settings`, package/version, profile/default features, manifest/lock names, input inventory and portable source mappings. SSR entries have `service: None`. The constructor remains private to verification. The inventory is descriptive, not permission to promote an executable.

Retain `Checked`; call `recheck()` at use boundaries. Stage through `stage_release()`, retain `Staged`, and copy through `Staged::consume()` so both source and stage are checked around consumption. Service metadata files are copied and revalidated with the executable. A consumer still owns destination validation, target compatibility and runtime policy. Do not reconstruct trust from stage.json.

Source reproduction, build isolation, approval, origin authorization and runtime remain not checked. A02-WEB remains open pending integrated runtime and real acceptance. No release state machine changes belong to this capture patch.

## Validation handoff

Authored fixtures cover capture/staging, independent-pin substitution, original and copied lock mutation, input modes/membership, path redaction, explicit registry/git rejection, legacy-service rejection, and static format-1 compatibility. Rust tests, build/check/clippy and heavy acceptance were not run in this implementation slot. Only formatting and cheap repository checks were run; see the implementation handoff for exact commands and results.

## Declaration association correction

Capture now checks a configured path against the actual selected Cargo manifest before replacing its portable path with `source`. Consumption repeats that check against the current project and copied source mapping. Package declarations still require the selected package name. The source must remain inside the project and name `Cargo.toml`.

The captured lock must be the `Cargo.lock` beside the captured workspace manifest and contain the selected local package/version without a registry/git source. A package's own workspace or explicit `package.workspace` must match that manifest; otherwise the package must be inside the recorded workspace directory. These identity checks do not reproduce Cargo's complete workspace/dependency resolution or prove that the backend was built from those inputs. Full source reproduction remains separate.

Regression fixtures include two different package directories with identical manifest bytes, absolute declared paths, substituted lock locations, explicit workspace mismatches and registry/other-package lock entries. The new Rust fixtures are authored but not yet executed; full acceptance remains pending the allocated build slot.
