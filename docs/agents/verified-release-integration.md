# Verified backend and website image integration (A04)

This is an opt-in path for backend-only releases and the explicit website production profile below. Existing image builds remain legacy. The original operator-pin path remains available; independent backend approvals and version-two deployment policies can now enforce operator-managed authorization. An independently authenticated descriptor digest is the operator's trust root; a publisher-supplied JSON file and its adjacent hash are not authentication. SSH image observations are not runtime attestation.

## Commands

The recipe and inventory use the existing backend verification format. Select real builder and runtime digests independently; do not substitute tags.

```sh
lk verify build --recipe recipe.json --out verified-build
lk verify reproduce --bundle verified-build/bundle --manifest verified-build/inventory.json --sha256 "$INVENTORY_PIN"
lk infra build-verified --bundle verified-build/bundle --manifest verified-build/inventory.json --sha256 "$INVENTORY_PIN" --runtime-image "$RUNTIME_IMAGE_AT_DIGEST" --runtime-config-sha256 "$RUNTIME_CONFIG_PIN" --tag registry.example/backend:unique-version --service backend-production --out descriptor.json
```

Packaging performs full reproduction again; it does not import a reproduction claim from JSON. Offline reproduction and supplied checkouts remain useful verification commands, but cannot authorize managed packaging. Runtime config pins hash the compact, recursively key-sorted JSON of the engine inspect object's Config member (SHA-256). Select that configuration deliberately: environment, healthcheck, shell and other inherited settings remain bound to the pin. ONBUILD and volumes are rejected.

The private packaging context contains only the executable and generated Dockerfile. Its instructions are FROM, COPY, WORKDIR, USER, ENTRYPOINT, CMD. No application Dockerfile or RUN instruction is accepted. Image inspection checks the complete inherited/modified configuration and platform. Extraction checks executable bytes and mode 0755. The image filesystem is also checked for unsupported /app/services. No descriptor is written on a failed check. The successful descriptor reports an engine image ID, not a registry digest.

The same image/configuration/extraction checks apply with `--reuse-existing`, which inspects an existing local tag instead of building it.

For managed Kamal deployment, pass `--service` with the destination’s exact `infra.name`. Kamal requires this service label on the image. Packaging binds it into the image configuration hash, and reuse checks it; do not add or replace the label after signing a descriptor. Different managed names require separately packaged images even when their executable bytes match.

Publication is a separate operator action; acceptance testing must not push. After deliberately publishing the checked image, authenticate the descriptor digest independently, then deploy:

```sh
lk deploy --version unique-version --verified-descriptor descriptor.json --descriptor-sha256 "$DESCRIPTOR_PIN"
lk deploy --rollback previous-version
lk deploy --recover --version previous-version
```

Use existing setup/maintenance/rollback-schema flags as required. Those checks are unchanged. History records the descriptor together with the observed registry digest. Pulling/reusing a recorded tag must match both; new releases check the independent descriptor before traffic changes and again before history advances. Once history contains verified evidence, it cannot downgrade to a legacy record. Legacy history deserializes with absent evidence, never invented verification. Shared history retains the existing operator trust model; this is not an append-only signed transparency log.

## Scope and acceptance gaps

The original backend-only recipe accepts projects whose sole platform is server; the explicit website profile below extends that path. Schema fingerprints come from migrations inside the frozen verified source. Source, recipe, dependencies and every bundle entry are covered by the independent inventory. Website assets require the separate production profile below; application service releases remain rejected. PR #553 requires a later descriptor covering every service executable and schema; app-only evidence must not authorize it.

Here, service-bearing means additional application services. The cache and relay configured by `infra.services` run in separate infrastructure containers and do not add executables to the backend image. They may accompany a verified backend, but their images, deployment configuration and live state are outside this descriptor's evidence.

RPC export runs the checked image executable with no network, a read-only root, no capabilities, a non-root user, bounded resources, and a 30-second deadline. The executable must export before database/auth startup; arbitrary executable behavior is not attested. The descriptor hashes the exact exported spec bytes after parsing. Without pinned baselines, compatibility is explicitly `not_checked`; the version-three structural comparison described below is opt-in. Only an explicit `--no-rpc` operator declaration records RPC as `not_applicable`. Native/guest compatibility and backend-only assets are `not_applicable`; services are unsupported; runtime attestation is absent. No online schema compatibility claim is made.

The real-engine fixture is `crates/leptoskit_cli/tests/fixtures/verified-backend`, a dependency-free executable that exports one query and one channel before a deliberate panic on normal startup. Run `devenv shell -- python3 docs/agents/verified-backend-acceptance.py <CLI>` after committing it, with `CONTAINER_CONNECTION` selecting a disposable local Podman VM. The script prints the builder/runtime digests and descriptor, independently hashes the extracted image executable, derives its pinned RPC baseline from the actual candidate export, and checks rejection without success evidence. It exercises a local committed source with a process-local Git transport mapping, not public provenance authentication.

## Follow-up validation

The follow-up rebases on `23e5a1df`. The CLI library suite passed: 559 tests, 5 ignored. A tiny dependency-free locked backend fixture exports before a deliberate normal-startup panic. `verified-backend-acceptance.py` uses a signed synthetic commit of its committed tree and an invocation-only Git transport mapping to exercise full reproduction locally; this does not authenticate public GitHub provenance. It never publishes images or deploys.

The initial RPC descriptor slice emits unsigned operator evidence. An independently obtained descriptor pin authenticates the selected bytes; it does not establish runtime attestation. The later approval and website sections below add signatures over the complete descriptor and reproduced website evidence. Application services remain unsupported.

Review found the initial packaging implementation reused a text checksum for executable bytes. The follow-up uses raw-byte SHA-256 and tests invalid-UTF-8 substitutions. It also normalizes Podman image IDs and validates a descriptor before writing it.

## Real container acceptance, 7 November 2026

Passed with Podman 6.1.4, a disposable libkrun VM (2 CPUs, 2 GiB RAM, 10 GiB sparse disk), and the committed dependency-free fixture. Two identical builds, full local-source reproduction, packaging, confined RPC export, independent image executable extraction/hash, and reuse passed. Executable/source/migration/config/recipe substitution, an incorrect runtime-config pin, and a replacement image executable retaining the schema label were rejected without writing a descriptor.

- Signed fixture commit: `661142d0b3efe3f7554185f494be64310c136782` (local synthetic commit, not published).
- Builder: `docker.io/library/rust@sha256:4333721398de61f53ccbe53b0b855bcc4bb49e55828e8f652d7a8ac33dd0c118`.
- Runtime: `docker.io/library/alpine@sha256:87a4361f6c819618a6bafa2ed5228714cb0bb22195d2b49c95eb26799990128a`.
- Inventory SHA-256: `dc788ff2da6b8f19ec0856f69fcc3b05aa2280970ba2400213dcc51b2df92060`.
- Built, packaged, and independently extracted executable SHA-256: `4678f8fcf4ac326bf75c0b200415444699fa29076ba4cde4b9d287b4da1b3517`.
- RPC spec SHA-256: `c34d10e5266c0f17b0b1a06fce68bdbd5a2cddab8fae213c0d1ea87c7121029d`; compatibility `not_checked`.
- Engine image ID: `sha256:0cbb253ba208b8c13d74657da2b882b9064bb15c58867ba9009e615e6a6ba0ad`.
- Descriptor SHA-256: `c72a5752f11ae8971e200015913e1feaf87195add5161b88bea89fdd4a04b690`.

The final output was `REAL ENGINE ACCEPTANCE PASSED; local fixture transport, no public provenance claim`. The false-label substitution failed with `image executable bytes differ`. No registry push or deployment occurred. This accepts the local backend packaging slice. Later sections record separate website and approval acceptance; application services and the broader A04 runtime/provenance requirements remain open.

The final CLI library run passed 560 tests (5 ignored). Full-workspace validation also exposed two pre-existing RPC test setup issues: the offline status test needed a reactive owner with sandboxed arenas, and the slow-reader test relied on a fixed sleep instead of observing each transition. The follow-up scopes the owner and synchronizes backpressure generation against the server counter; it preserves the full-transition assertion.

## Post-rebase command acceptance

Rebased onto `7fa1b202bb1004e7a49035f21f7f86fb38737efc`, retaining backend configuration, readiness declarations, and auth readiness. Main already contains the offline live-client Owner repair, so the duplicate commit was dropped.

The fake remote command suite now exercises independently pinned descriptors through `lk deploy`: image mismatches before proxy/app boot, postboot mismatches stopping applications without advancing history, recorded rollback and explicit recovery, automatic recovery verification failure, and successful recovery retaining the descriptor. Invalid digest fields, source recipes, runtime pins, and inconsistent target/platform pairs are rejected before boot. Shared history rejects unknown fields, malformed descriptors, and legacy records after verified records rather than rewriting away evidence. Automatic recovery failure stops applications.

These are local command tests with fake SSH and container observations, not real deployments or runtime attestations. The real Podman packaging evidence above predates this rebase; container build, extraction, and RPC execution are unchanged. No additional VM or registry publication is required for these release-boundary and descriptor-validation changes.

Post-rebase focused validation passed: 561 CLI library tests (5 ignored), 45 infrastructure command tests, and 14 help/scaffold snapshot tests. The final three verified-release command tests passed again after strengthening the automatic-recovery stop assertion. The full normal pre-push gate was attempted twice; external removal of this clone’s build output and the shared Rust toolchain interrupted Clippy (exit 255, then SIGTERM/143). A completed gate remains required before republishing.

## PR568 completed gate

PR568 merged as `f7419898aaa71b3cdde3d6fb233504660e2789ce`. Its published signed head `de668724a3248097f2ed7aefd2325cad48fde97f` passed the unmodified full pre-push hook: 4,821 workspace tests passed (48 skipped), plus 16 file-routes tests and 43 DevTools tests. Clippy, formatting, docs, Kotlin, release metadata, schema, workflow security, secret scanning and dependency policy passed. This supersedes the interrupted-gate paragraph above. Packaging execution was unchanged after the recorded Podman acceptance.

## Independent backend authorization

The operator installs `backend-trust.json`, `backend-authorization.json`, and the current DSSE `backend-deployment.json` under `infra/<deployment-name>/`. These files are local operator state, not configuration accepted from a release publisher. Protect and back up this directory, including `backend-policy-state.json`; deleting or replacing replay state defeats local replay protection. Existing local infrastructure locking serializes policy state updates. This is not cross-operator consensus or a transparency log.

Trust format version 1 names `repository`, `deployment`, `publisher` (hex public key), `approvers` (hex public keys), `threshold`, and `deployment_keys`. Key aliases are compared as decoded bytes. The publisher key is excluded from approvers. A configured trust file fails closed on malformed, missing, legacy or unsigned authorization; previously approved history cannot return to pin mode.

Authorization format version 1 contains `descriptor` (the original JSON as a string), `approvals` (the objects emitted by `verify approve-backend`), and `deployment_policy` (the signed DSSE envelope as a string). Deployment replaces the latter with the current operator-selected `backend-deployment.json`, then stores that envelope with the original descriptor and approvals in history. Nothing deserialized is a trusted verification token.

Approval statements use `leptoskit-backend-release-approval-v1` followed by a NUL as their signing domain, distinct from guest approvals. Exact statement bytes are signed using Ed25519, following code-push's exact-text convention. The versioned statement binds the SHA-256 of the entire original descriptor and repeats the repository and commit to reject conflicting source commitments. Consequently inventory, recipe, executable, image config/content, migration and RPC evidence are all covered. Embedded keys are only identifiers: only independently configured approvers count.

Deployment policy version 2 adds `backend: {descriptor_sha256, engine_image_id}` to each release tuple. Version 1 remains readable but cannot carry this binding or authorize managed backend approval mode. The existing DSSE verifier checks pinned deployment keys, canonical origins/repository, expiry and sequence/equivocation state. Current policy must explicitly list a rollback/recovery target; operators issue a higher sequence policy retaining an old target rather than disabling replay checks. Automatic recovery also revalidates the current policy and approvals before starting the old image.

Approvals authenticate a reviewer's statement, not code safety or a running executable. Website releases use the same complete-descriptor approval binding. Application services remain unsupported, RPC compatibility defaults to `not_checked`, and runtime attestation remains absent. The content-addressed engine image ID and descriptor config digest bind the image; the remote registry digest is observed and retained separately as before.

## Website production profile (A04 continuation)

The explicit `build.json.website` object selects `leptoskit-web-v1`. It requires `profile`, `config_sha256`, `toolchain_sha256`, and `output_name`. The configuration pin hashes the raw committed `leptoskit.toml`. The toolchain pin hashes the exact concatenated stdout of `rustc -vV`, `cargo -V`, `leptoskit --version`, and `wasm-opt --version`, in that order, in the selected builder and source toolchain. The immutable builder image must contain these tools and the Wasm Rust target. Its digest binds the CLI's embedded wasm-bindgen, splitting/compression libraries, operating system and every installed tool; the version report is an additional consistency check, not a substitute for that image digest.

Supported production profiles are full-stack SSR and CSR, with a Linux GNU backend on a native aarch64 or x86_64 builder and a wasm32-unknown-unknown client. The backend package and binary must both equal `app.name`. Configure build flags and features in the pinned application configuration: backend recipe overrides must be empty/false. Version one uses the normal bundle settings, including split detection, optimization and compression, but requires no explicit plugins and no localization. Islands, static generation and external plugins are rejected. Native outputs are not built or certified even if the application enables native platforms. Application services remain unsupported; infrastructure cache/relay declarations under `infra.services` do not count as application services. Both profiles share R07’s root-key classification and preserve its distinction between infrastructure containers and application executables.

The builder runs the ordinary `leptoskit build --release --platform web` pipeline in a fresh writable source copy, with no network, read-only dependency mount, read-only container root, fixed time/locale and two Cargo workers. Source inputs remain frozen. The generated source copy must retain the original Cargo.lock. The normal pipeline selects server/client features, compiles the backend and Wasm, applies embedded wasm-bindgen, runs configured wasm-opt and compression, and copies HTML, CSS and public files. Committed generated output directories and public/web `pkg` directories are rejected so static files cannot replace the generated browser module. Two clean builds must produce the same executable and complete site tree; reproduction repeats these steps from authenticated inputs. Local checkout and invocation-only local Git mappings remain explicitly local source evidence, not authenticated public provenance.

Website files live in `artifacts/site` and the ordered toolchain report in `artifacts/website-toolchain.txt`. Website trees require regular files with mode 0644 and directories (including the root) with mode 0755; links, special files, unsafe paths, missing JS/Wasm and extra or missing tree members fail. Backend-only inventories retain their existing format and assets remain `not_applicable`. `record`/`check` are still integrity checks only: supplied assets do not acquire source reproduction evidence just by receiving an inventory.

Packaging copies only the frozen executable and validated site tree. The application Dockerfile, sources and package managers do not enter the context. There is no recompilation or RUN instruction. The exported image is parsed without unpacking its tar onto the host; the complete `/app/dist` tree must match the reproduced paths, types, modes, sizes and raw-byte hashes, including rejection of inherited extra assets. Reuse runs the same checks. No descriptor is written after a container or validation failure.

Website descriptors without checked RPC evidence use format version 2 and retain the managed image descriptor kind and signing domain. Their `assets.reproduced` object carries the profile, toolchain digest and complete tree. Version-one backend descriptors remain readable; a website recipe cannot use `not_applicable` or missing asset evidence. Independent approvals cover every descriptor byte, and the version-two policy binds that descriptor digest plus the image ID. The shared release gate checks application kind during deploy, rollback and recovery. Remote image inspection remains an observation; RPC compatibility defaults to `not_checked`, and runtime attestation remains absent.

### Website acceptance procedure and current status

`verified-website-acceptance.py <CLI> <builder@sha256:...> <runtime@sha256:...>` uses the committed `verified-website` fixture through the real production CLI. It independently compares extracted asset hashes/modes and the backend, checks reuse, re-records modified inventories to distinguish integrity from reproduction, and rejects image asset substitution retaining the schema label and configuration. The tiny fixture has the ordinary full-stack Cargo layout, a real wasm-bindgen client, static CSS/HTML/public files and a small backend that exports its RPC contract before opening a socket. It does not claim full UI/browser acceptance.

The focused library, infrastructure command and snapshot run passed 648 tests (5 existing skips), and CLI Clippy passed with warnings denied. The implementation is rebased onto `68dde5567d6158db6c50028e28674ce7e69b6256`. The unmodified full pre-push gate passed: 5,036 workspace tests, 16 file-routes tests and 43 DevTools tests, with 47 existing skips. Clippy, readability, examples, docs, formatting, Kotlin, workflow security, release metadata, schema and dependency policy passed.

Real Podman acceptance passed with the committed fixture, default split detection, wasm-opt and compression enabled. The complete descriptor and independently extracted tree are recorded in [verified-website-acceptance.json](verified-website-acceptance.json). The source fixture commit is signed but local, using an invocation-only Git transport mapping; it is not authenticated public GitHub provenance. A loopback registry inside the task-owned VM held the pinned builder; no application image was published. The VM used 2 CPUs and 3 GiB RAM and was removed after acceptance.

The builder's CLI was compiled from main `08131a29` with locked dependencies in the pinned Rust 1.95.0 image. The released CLI predated `build.backend`, so it could not serve as this fixture's builder. The final immutable builder includes the compatible CLI, wasm32 standard library, and checksum-pinned Binaryen 125. The JSON record contains their exact digests and the source archive hash.

- Builder: `127.0.0.1:5007/a04-builder@sha256:f302068ace1063152f0be8c87bb9b190d2049d0dcdc4046de781a6af7f3d1d4e` (local fixture registry, now removed).
- Runtime: `docker.io/library/debian@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc`.
- Inventory: `673d138d1bff88c25a9f18588f605277b4307fe9ffc1595ce2fb2723d7a301db`.
- Backend: `6c33900444f30b2276485e18adba51325c92286cc98285d733b40fae9c4e52b8`.
- Descriptor: `4521b8bb69cdf8e3fc37d216d4db7eece201dea01994c9e8dc0c439207a43cb7`.

All 13 site members matched independent extraction, including generated JS/Wasm, compressed variants, HTML, CSS and static files. Re-recording changed asset inventories did not authorize their bytes: source reproduction rejected them. Source/configuration changes, tree addition/deletion/modes, a self-consistent replacement toolchain report, incorrect runtime pin, missing engine image and a replacement image asset retaining the schema label were rejected without a success descriptor. The final output was `REAL WEBSITE ACCEPTANCE PASSED`.

Earlier validation was interrupted by external removal of the protected target, shared Rust toolchain and the task-owned VM disk. The completed focused run and real acceptance supersede those interrupted attempts. A subsequent cache/toolchain removal during the full gate required an isolated pinned Rust installation and target under the exclusive clone’s `.git/a04-rustup` and `.git/a04-target`; surviving logs are preserved in `.git/a04-evidence`. No checks were weakened.

The actual extracted-image descriptor was also signed with a throwaway local test key through `verify approve-backend`. Independent OpenSSL Ed25519 verification passed, and replacing the descriptor digest in the signed statement failed verification. The test key is not human review or deployment trust. The acceptance JSON preserves the original descriptor text and this public test approval.

## Structural RPC compatibility integration

`build-verified` accepts paired, repeated `--rpc-baseline` and `--rpc-baseline-sha256` arguments. Each independently pinned file is read once before reproduction, with at most eight baselines and 16 MiB per spec. Packaging still exports the candidate spec by executing the checked image. No other build or source export substitutes for that executable.

Descriptor format 3 retains the exact candidate spec, baseline specs and hashes, and `leptoskit-rpc-structural-v1` algorithm identifier under `rpc_contract.compatibility.checked`. Readers verify hashes and recompute `ApiSpec::breaking_changes`; a serialized success flag cannot bypass the comparison. Backend and website asset rules are unchanged. Unchecked descriptors retain formats 1 and 2. Unknown algorithms, empty or duplicate checked baselines, and inconsistent descriptor versions fail validation.

Operator trust format 2 requires a nonempty `rpc_contracts` array of independently authenticated baseline hashes. The shared approval verifier requires every hash in the candidate evidence during deployment, reuse, rollback, and recovery. Publisher-selected baselines cannot weaken this window. Trust format 1 remains the explicit legacy policy and cannot contain nonempty contract requirements. The current operator window applies to old images too: an otherwise valid historic approval does not establish compatibility with newly supported clients. Recorded rollback and recovery retain their original evidence. If that lacks a newly required baseline, operators must reproduce the old source and issue fresh evidence, approvals, and deployment policy under a new release tag; they cannot replace a recorded tag's descriptor.

The existing exact-descriptor approval and version-two deployment policy cover the new fields without a new signing domain. They authenticate the declared structural evidence; neither the signatures nor the comparison prove handler behavior, authorization, database rollout compatibility, runtime protocol behavior, or runtime attestation.

Targeted validation on 2026-11-07 passed 87 codegen tests with default features (one existing skip), 33 without defaults, 685 CLI unit tests (five existing skips), and 92 CLI integration tests (two existing skips). Actual clap help matched the committed snapshots without accepting new snapshots. The eleven captured R16 command cases were rerun with freshly compiled CLI SHA-256 `5ba6742c428d2dc77e6e0693d3befacba41db64b19a8c1409cd5fbfa8d3cabdf`: all ten changed contracts failed, and the unchanged control passed. Review additionally fixed the generator-only doctest for feature-independent builds and added schema traversal for `contentSchema`, with tuple/dependency regression coverage.

Real Podman backend acceptance passed reproduction, packaging, independent executable extraction, checked RPC/reuse, changed-pin and self-consistent incompatible-baseline rejection, `--no-rpc` conflicts, and independent OpenSSL approval signature verification. The checked descriptor SHA-256 is `a14794cf0f2f04188b23a556e6791352b63faf6017646e98df396e85e6a5157d`; its executable SHA-256 is `4678f8fcf4ac326bf75c0b200415444699fa29076ba4cde4b9d287b4da1b3517`. Complete local fixtures, recipes, pinned image metadata, exact descriptors and logs are retained in the exclusive checkout's `.git/rpc-evidence`; set `LEPTOSKIT_ACCEPTANCE_EVIDENCE` to a new directory to retain future script runs. The task-owned 2-CPU/3-GiB VM was removed afterward.

The real-engine fixture now exports a nonempty `list_items` query and an `updates` channel. `verified-backend-acceptance.py` first observes that exact export through an empty bootstrap baseline, then packages again with the observed bytes as an independently pinned baseline and requires reuse to pass. It rejects self-consistent baselines that remove a prior `legacy_list` function, change `list_items`' result schema, remove a prior `legacy_updates` channel, or change `updates`' message schema; it also rejects appending a byte without re-pinning and combining `--no-rpc` with a baseline. Approval still covers the complete descriptor, and its Ed25519 signature is checked independently with OpenSSL. These are structural declaration checks: they do not prove RPC endpoint behavior, authorization, public provenance, cloud deployment, or runtime attestation.

The earlier local attempt could only check Python syntax, a host fixture export and the installed CLI's comparator: no Podman engine was available and the filesystem was full. The completed engine run below supersedes that limitation for the nonempty backend fixture. Website checked-descriptor approval paths passed unit tests, but the website builder was not rerun for new checked-RPC packaging; earlier real website proof remains specific to its recorded version. The historical 5,160-test workspace gate above also belongs to its recorded revision, not the strengthened fixture's final branch gate.

### Nonempty RPC image acceptance, 11 November 2026

The strengthened script passed on Mini02 with exit 0 and both terminal pass markers. Its signed harness commit was `31492e3992b7bc2b58a2ee6f3499504e099180c5`; full tree `1b9c05b6b0a72fab48638906f8c2c92851019938` exactly matched the parent implementation at `77d835da01ff2d1e5c517a329070ec519ce82a19`. No fixture or script corrections were needed. The source fixture used by reproduction is a separate synthetic signed commit of the fixture subdirectory.

- The exact-source CLI's SHA-256 was `41bd41da7a2b1210074e698f60d7d506002a47ee9009bc1dc8b4443a1f567a65`.
- Builder: `docker.io/library/rust@sha256:4333721398de61f53ccbe53b0b855bcc4bb49e55828e8f652d7a8ac33dd0c118`.
- Runtime: `docker.io/library/alpine@sha256:bc21487101ae47172509130ee1092cc273a831968d64b617776d1f69c81e42ab`.
- Checked descriptor SHA-256: `a1139d5f682bf5812729d19b62fcccf548fc30dd2b5018c134dcb25e85d31543`.
- Independently extracted executable SHA-256: `19ba7310c2034a8e608ac6821b8b29dc9e66eabde8c24b25843539e5929cbe5a`.
- Exported RPC and successful baseline SHA-256: `e83869b28f83742e29f55d555da209684e2f654bcfdbc0162913455c139bff1d`.

Reproduction, image extraction, successful nonempty baseline packaging/reuse, all four structural incompatibility cases, stale pins, `--no-rpc` conflicts, artifact/image substitution and independent OpenSSL approval verification passed. Podman client/server were 6.1.4 with libkrun; the task-owned VM used 2 CPUs, 3 GiB RAM and 10 GiB disk. Cleanup removed that VM and its connections, restoring the initially empty machine/connection lists.

Evidence is retained in `/Users/mini02/Developer/rpc-acceptance-evidence-20261111` and copied to the parent audit's `mini02-engine-evidence` directory. The SHA-256 of `SHA256SUMS` is `13bcb0cb7df8c4679e03129d5f16411fe0a45123090b9e481229bf8ac4d097d2`; the parent independently rechecked this pin and all 76 listed files after copying. This is structural declaration and local image-engine evidence. Invocation-only local Git mapping and throwaway approval keys do not authenticate public provenance or production trust, and the fixture does not establish endpoint behavior, authorization, cloud deployment or runtime attestation.
