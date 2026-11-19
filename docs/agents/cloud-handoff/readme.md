# LeptosKit Cloud: handoff

Written 18 November 2026. This folder hands the LeptosKit Cloud build to another agent. It is temporary: delete it once its contents have been folded into `docs/agents/cloud.md` and the work has landed.

## The goal

A hosted platform for any leptoskit app. A developer signs up, runs `lk cloud login` and `lk cloud launch` (or presses one button), and gets their server, database, web client, over-the-air updates and store submissions running on our infrastructure, driven by the `leptoskit.toml` they already have. The owner also wants Kubernetes and microservices available in it, the lowest infrastructure cost we can credibly run, and store publishing beyond Apple and Google (Solana dApp Store, Huawei AppGallery with HarmonyOS, and others).

Read, in this order:

1. [`cloud-architecture.md`](cloud-architecture.md): the contract every slice was built against. §13 lists the slices; §14 holds the amendments made after the design direction.
2. `docs/agents/cloud.md`: the engineering note in the repository, with the status of each slice, the permission table, the worker protocol and both security-review sections.
3. `docs/agents/cloud-design.md`: the console's design direction (the release sheet you sign and the cloud stamps). §19 lists gaps in leptoskit; §20 lists what the screens need from the API.
4. `docs/agents/cloud-platform.md`: the operator runbook for the cluster.
5. The research: [`research-infra-costs.md`](research-infra-costs.md), [`research-cloud-product-flows.md`](research-cloud-product-flows.md), [`research-store-apis.md`](research-store-apis.md). Prices are dated list prices, not quotes.
6. [`codebase-map.md`](codebase-map.md): how this repository builds services, RPC, auth, config, store clients and its tooling.

## What is on GitHub

Every branch below is pushed. They form one linear stack on `main` at `511655984`, in merge order: each branch contains everything below it.

| #  | Branch                           | Tip         | What it is                                                                                                                                                                                                                                                              |
| -- | -------------------------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1  | `docs/cloud-design`              | `b1fc235fc` | The console's design direction.                                                                                                                                                                                                                                         |
| 2  | `feat/cloud-platform`            | `9f89ce0b8` | `leptoskit_cloud_platform`: typed platform config, OpenTofu JSON, two-phase k3s node bootstrap, the operator command.                                                                                                                                                   |
| 3  | `feat/cloud-foundation`          | `231f3208d` | `leptoskit_cloud` shared types (slugs, sizes, deployment states, source archive, price estimate) and the `[cloud]` config table with its schema.                                                                                                                        |
| 4  | `feat/cloud-platform-addons`     | `1d87eb051` | Cluster add-on manifests (gVisor, CloudNativePG, KEDA, cert-manager, Traefik, zot, policies) and the runbook.                                                                                                                                                           |
| 5  | `feat/cloud-runtime`             | `7f0db7fc1` | `leptoskit_cloud_runtime`: `ReleaseSpec`, the `Runtime` trait, Kubernetes manifests, a recording fake.                                                                                                                                                                  |
| 6  | `feat/cloud-runtime-cluster`     | `906d76dc0` | Corrections (rollout quota, KEDA `InterceptorRoute`, agreement with the platform) and the cluster adapter over `kube`, with its RBAC.                                                                                                                                   |
| 7  | `fix/rpc-app-server-feature`     | `d70bf63e0` | Two `leptoskit_rpc` fixes: app crates gate their operations behind their own feature and `#[api(scope)]` is kept on operations; client service helpers compile only where used.                                                                                         |
| 8  | `feat/cloud-control-accounts`    | `2c81180ad` | Control 1: sign-up, organisations, members, invitations, API tokens, projects, environments, audit log, the `leptoskit-cloud-control` server, and the fixes for its security review.                                                                                    |
| 9  | `feat/cloud-cli-accounts`        | `2e9376376` | `lk cloud login` (device flow) and the `whoami`, `org`, `project`, `env`, `token`, `link` commands. Also makes `leptoskit_cloud`'s tables server-only and fixes the binary dropping the identity provider's routes. Includes three `wip(cloud)` commits to squash away. |
| 10 | `feat/publish-solana`            | `3706b9c4d` | `lk publish solana`: the Solana dApp Store through Solana Mobile's official tool.                                                                                                                                                                                       |
| 11 | `feat/cloud-control-deployments` | `b9828ad7b` | Control 2: uploads, the deployment lifecycle with release-sheet lines and stamps, logs, the worker protocol, operator operations. Reconciled onto the stack by cherry-pick; **the reconciliation was interrupted** (see below).                                         |
| 12 | `docs/cloud-handoff`             | this branch | This folder, on top of 11.                                                                                                                                                                                                                                              |

One branch is **not** on the stack:

| Branch                     | Tip         | What it is                                                                                                                                                                                                                                                                                                        |
| -------------------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `feat/cloud-console-shell` | `e43ec3140` | UI 1: the console's shell, sign-in, welcome, device approval, organisations, account, projects list. Server-rendered and hydrated with leptoskit, no JavaScript. Its base is an older state of layer 8 (before the security fixes and layers 9 to 11), so it must be cherry-picked onto the stack and reconciled. |

Draft pull requests are open for the stack; each targets the branch below it.

## What has been verified, and what has not

- **Each slice's own tests and lints** passed when its builder ran them (counts are in `docs/agents/cloud.md` and in each pull request). Control 1 and Control 2 ran their Postgres scenarios against a real Postgres.
- **The full pre-push suite (`devenv shell lint:push`) was run once on layer 10 (`3706b9c4d`)** in an isolated worktree. gitleaks, workspace clippy (native and wasm), monostyle, formatting, docs and the Swift and Kotlin lints all passed. The test run was stopped for this handoff after 2,030 of 7,610 tests had passed with none failing. **The remaining tests, the dependency audit and the schema check were not reached.**
- **Layer 11 has not been through the full suite at all**, and its reconciliation was cut short: the cherry-pick committed cleanly and one follow-up edit to `crates/leptoskit_cloud_app/tests/binary.rs` is in the `wip(cloud)` commit on top. Nothing was re-run after the cherry-pick. Check in particular that every table it adds sits in the server-only module, that `crates/leptoskit_cloud_app/src/server/mod.rs` still applies the auth and identity-provider layers last with nothing merged afterwards, and that `tests/binary.rs` and `tests/cloud_cli.rs` both pass with the storage settings Control 2 requires.
- **Nothing has run against real infrastructure.** No cluster was provisioned, no store was contacted, no payment was taken, and `tofu validate` did not run (OpenTofu was not installed where the platform crate was written).
- These branches were pushed with `--no-verify` at the owner's request. Treat the pre-push suite as not yet passed for any of them.

## What to do next, in order

1. **Land the stack.** For each layer from the bottom: run `devenv shell lint:push`, fix what it finds in that layer, squash-merge its pull request, then rebase the layers above onto the new `main`. Squash the three `wip(cloud)` commits in layer 9 and the one in layer 11 into their neighbours first. `main` moves often (other agents push); rebase rather than merge.
2. **Finish reconciling Control 2 (layer 11)** as described above, then fix the nine findings in [`control2-security-review.md`](control2-security-review.md) (three high, six medium), each reproduced as a failing test first, and add a "Security review" section for them to `docs/agents/cloud.md` the way Control 1's was recorded. Also close the test gaps that review names (the matrix with cluster-worker identities; log redaction across lines; terminal control characters in log text).
3. **Fix the web renderer bug that blocks the console** (`docs/agents/cloud-design.md` §19, gap 15). leptoskit's web renderer builds `main`, `nav`, `ul`, `li`, headings and other semantic tags through tachys' custom-element constructor, which reuses an element only when its tag equals the DOM's upper-case `tagName`, so every render replaces those elements and a text field loses focus after one keystroke. Fix it in `crates/leptoskit` / `crates/leptoskit_native` with typed tachys elements, with a browser test that a semantic container keeps its DOM node and focus across a re-render, as its own pull request against `main`. A branch `fix/web-semantic-elements` was started and has no commits.
4. **Reconcile UI 1** (`feat/cloud-console-shell`) onto the stack. It restructured `crates/leptoskit_cloud_app/src/server/mod.rs` to serve the pages; keep its version of the router shape with the layering rule intact.
5. **Build the remaining slices** in the contract's §13: Control 3 (variables, the secrets vault, domains, managed Postgres, which lifts the "managed Postgres is not available yet" refusal in `deploy`), Control 4 (metering, Stripe, plans, quotas, the spend cap, rate limiting), Worker 1 to 3 (`leptoskit_cloud_worker`: the claim loop and build role, the cluster role using `leptoskit_cloud_runtime`, then native builds and store publishing), CLI 2 and 3 (`launch`, `deploy`, `status`, `logs`, `open`, then `secret`, `domain`, `scale`, `rollback`, `db`, `publish`, `usage`), UI 2 and 3, the remaining stores, the price catalogue in the platform crate, an end-to-end local stack test (control plane, local runtime adapter, local build worker, `lk cloud launch` of an example), and the public guide chapter.
6. **Remaining stores**, one pull request each, in this order: Huawei AppGallery for Android and HarmonyOS, Microsoft Store, Samsung Galaxy Store, Meta Horizon Store, itch.io and Steam, then Amazon Appstore and the Snap Store. [`research-store-apis.md`](research-store-apis.md) has the endpoints and what stays manual; [`solana-brief.md`](solana-brief.md) shows the level of detail a store brief needs.

## Known gaps and requests from the builders

- **Framework:** any leptoskit mutation that returns a secret stores it in its idempotency receipt. The cloud avoids it by using actions; `leptoskit_rpc` should get an option such as `#[rpc::mutation(receipt = "outcome")]`.
- **`leptoskit_db`:** Postgres returns `SUM` of a `bigint` as `numeric`, which it cannot decode; a second database handle inside a mutation on SQLite hangs instead of erroring.
- **`leptoskit_cloud_runtime`:** needs `JsonSchema` derives and a serde `ClusterProfile` (the control plane mirrors both today), and cookie stripping on routes (public isolated services are refused until then).
- **`leptoskit_config`:** `ResolvedCloud` should derive `Deserialize` so the control plane can store it.
- **Device flow:** it has no client-description field, so the release sheet cannot yet say "signed from the terminal on <host>".
- **Audit log:** append-only by convention; the migration tooling cannot express a database rule for it.
- **Cluster worker RBAC:** Kubernetes cannot scope a ClusterRole to namespaces that do not exist yet, so the worker can list Secrets cluster-wide and must be treated as trusted.
- **Console:** six smaller leptoskit gaps are recorded in `docs/agents/cloud-design.md` §19 (document title, remembered appearance, a focus variant, keys with nothing focused, axum 0.8 route paths, text wrapping in a row on the web).
- **`chrono`** is flagged unmaintained and excepted in `deny.toml`; `leptoskit_db::Timestamp` is built on it, so replacing it is its own migration.

## Decisions waiting for the owner

1. **Apple builds on our own Macs.** Apple's macOS licence restricts running other people's builds on our hardware; it needs legal review before a macOS build pool is sold.
2. **Prices.** The proposal is a free Hobby tier and Pro at $20 a month with $20 of usage included, a Linux build minute at $0.005 and a macOS build minute at $0.04. Prices are configuration, not code.
3. **Domains.** The code defaults to `cloud.leptoskit.dev` and `*.leptoskit.app`; neither has been checked or registered.
4. **Console changing scale.** The design lets the console change scale and shows the drift from `leptoskit.toml`; the owner may prefer the file to be the only way.

## Rules that matter here

- Follow `AGENTS.md`. Interfaces are leptoskit components compiled to Wasm: no hand-written JavaScript; a gap in leptoskit is reported and fixed in leptoskit, never scripted around.
- In `leptoskit_cloud`, tables are server-only, every type a client sees is a plain serde type, and an operation never returns a table row. Every permission is decided in `server/authorize.rs` from the table in `access.rs`; managing members, owners and tokens requires a person, never an API token.
- Anything multi-tenant or credential-bearing gets an independent security review before it is merged. Both reviews so far found real defects.
- Docs state only what has been done for real. Nothing here may claim a provisioned cluster, a billed customer or a store upload from a cloud worker until someone has done it.
- No images in the repository. Commits are the owner's alone: no AI co-author trailer. The owner's commits are GPG-signed; if your environment cannot sign as the owner, say so before committing rather than changing any signing configuration.
