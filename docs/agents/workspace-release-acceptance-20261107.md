# Workspace release acceptance, 7 November 2026

Real local two-app release acceptance passed on signed implementation commit `a494884b84c5acb231b7f705ad5a32d74779e5c8`. This accepts the pinned, prebuilt-image orchestration slice; it does not close A02. No cloud resources, production credentials or public image registry were used.

## Source and environment

The original three good signed patches remain unchanged after their rebase onto `8051706d`: `013f32b0`, `cbb0682d`, and `19bc6d25`. Commit `36de6810` preserved the serving fixture and repaired normal-gate failures. Commit `a494884b` fixes numeric registry ports, verified image service labels required by Kamal, and Docker's null defaults when inspecting Podman image configurations. The exact image ID remains required; non-null configuration changes fail. Tests cover invalid registry authorities/tags and changed image configuration.

The parent restored and verified 4165 file hashes/modes for nightly `2026-10-21` at `/Users/mini02/Developer/leptoskit-workspace-protected-20261107/rustup`; this task did not alter it. Commands ran through `devenv shell`, with two Cargo jobs, no incremental compilation/development debug info, four test threads, and required Postgres/restic checks. No shared toolchain, target or unrelated process was removed.

## Actual release journey

The disposable VM used 2 CPUs, 3 GiB RAM and a 12 GiB disk. Actual Podman reproduction/packaging, Docker 28.5.1, a private local registry, OpenSSH, Kamal 2.12.0, Kamal proxy, and two independent Postgres containers ran locally. Each initial image was fully reproduced from the signed synthetic fixture source, packaged with its managed service label, independently pinned and approved with throwaway local keys. The website-only changed executable was independently reproduced and packaged under a second version.

Both apps deployed, then the same plan freshly revalidated both. Updating only website changed its image, container and history. Accounts retained exactly:

- Container `35bbd358fe5924b65184b96f003389758624aecaa676fdb13ddfece574f51f10`.
- Image `sha256:bc7df52f764e5a414d163219a2a5fec0907118eecc416f47bafceb11797cd6f3`.
- Start time `2026-11-07T21:39:58.360962615Z`.
- Release-history SHA-256 `446c7ed445cea407d0d63b35599c43f1acb1d9e3495efdfa94314ef39dd7afc4`.

The update-plan retry left both containers and histories unchanged, with fresh readiness checks. Independent final probes returned `ready: true` for both, and real SQL queries returned each database's empty migration history. The acceptance CLI SHA-256 was `80bc30739bfdaaf0e804270d243ae50f9cefe94b85c118f6d6383ed6f8c543f6`. The final source commit includes equivalent refactoring and formatting after that binary build.

Live negatives passed for rotated credentials, changed descriptor bytes, changed parsed application configuration, changed workspace bytes and plan pin, stopped readiness dependency, malformed/expired/revoked deployment policy, legacy history without configuration evidence and unfinished operation phase. A stopped consumer persisted a journal with accounts `command_completed` and website `failed`; explicit retry revalidated both. Real packaging refused a wrong service label and a self-consistently pinned incompatible RPC baseline without producing a success descriptor. The correct service-label reuse control passed.

The committed [evidence directory](workspace-release-evidence-20261107/readme.md) retains exact scripts, recipes, descriptors, before/after observations, partial/retried journals, final readiness/database observations and raw-log hashes. Scripts describe their local transport adaptations; they are recorded acceptance drivers, not a cloud provisioning product. Raw logs and fixture source bundles are at `/Users/mini02/Developer/workspace-release-acceptance-20261107`. Checksummed compact copies are at `/Users/mini02/Developer/leptoskit-workspace-protected-20261107/evidence` and in the clone's `.git/workspace-acceptance-evidence`. Private keys are excluded from committed evidence and secondary public copies.

## Boundaries and remaining work

Provider outputs, cloud-init completion and idle Cloudflare tunnel containers were explicit fixtures. Real Kamal received a local SSH port/key, arm64 selection and fallback version for a fixture outside a Git checkout. Its unnecessary operator-side registry login was skipped; remote registry login, pull, proxy/app boot, pruning and release operations ran normally. Git source transport mapped the public repository URL to local signed fixture commits; this is not public provenance authentication.

The backend is a tiny HTTP/RPC-export fixture. It does not use its database, authenticate callers or execute migrations. Separate real Postgres containers and actual empty histories establish the orchestration's observation boundary, not schema rollout or application-level database readiness. The RPC negative compares an empty actual export with a nonempty baseline; it does not establish nonempty handler compatibility. Deliberately stopping containers in negative controls changes their start times; the unchanged-app comparison is taken after those controls and immediately around the website update.

Full A02 still requires automatic builds/change impact, private bindings and caller identity, physical schema ownership, cross-app schema/API compatibility and generated-client acceptance. Apps are not one transaction. A timed-out local command may leave a remote reservation. Readiness is an operator observation, not runtime attestation or proof of public routing.

The task-owned VM, registry, images and containers were stopped and removed after final observations. `podman machine list` was empty. Protected Rust, shared caches and all evidence remain intact.

## Validation and publication

The first completed normal `lint:push` passed 5656 workspace tests (55 existing skips), 16 file-routes tests and 43 DevTools tests, plus all lint/schema/release/security/dependency checks. Real acceptance then exposed the registry/service-label/cross-engine issues fixed above. The focused registry command test, 16 verified-packaging tests, seven readiness HTTP/Python tests, schema regeneration (no asset diffs), Clippy and `fix:all` passed. The one help snapshot change was reviewed explicitly.

The final implementation’s normal `lint:push` passed 5658 workspace tests (55 existing skips), 16 file-routes tests and 43 DevTools tests, with all remaining gates passing. `lint-push-final.log` is pinned in the evidence manifest. Publication uses the same unmodified pre-push gate; its output is retained as `normal-push.log` outside the repository. No hook was bypassed. No CI watch or merge is part of this child task.

## Final main rebase

Before publication, main advanced to `0c66c58142338c3399198783a442bc777f04476c` with native badge/shortcut work. The superseded push was stopped before publication, then all six commits were rebased with signing. `git range-diff` reported every patch unchanged. The original workspace commits are now `d765c9c5`, `dd7f9cf6`, and `bf7c3a7d`; the fixture/gate commit is `a96ed8ce`, and the packaging fixes are `617bb4f1`. All reported good signatures. No deployment/reuse implementation changed in this rebase. Real-engine evidence above remains tied to its stated pre-rebase binary; the normal gate is rerun on the integrated tree.

The integrated normal gate passed on `0c66c581` with 5658 workspace tests, 16 file-routes tests and 43 DevTools tests, plus every remaining check. Three earlier integrated attempts received SIGKILL during compilation, including a one-job diagnostic retry; their cause was not established. The successful retry used a terminal-attached session and the original two-job environment. Cargo redownloaded registry contents during that retry; this task did not clear them. The protected compiler remained intact throughout. The successful log is `lint-push-rebased-pty.log`.
