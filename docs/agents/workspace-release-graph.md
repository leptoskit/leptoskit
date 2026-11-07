# Workspace release graph

The A02/X04 workspace layer references complete application configurations. It complements PR553's processes inside one app; it does not replace that app's service registry or pretend that Kamal roles sharing one image are independent artifacts.

The public contract is `leptoskit_config::workspace::Workspace`. `applications.<id>.path` resolves beneath the manifest root to an existing `leptoskit.toml`. `depends-on` is a readiness graph; `bindings` is a separate directed call graph and may contain cycles. Graph validation is deterministic. The CLI validates existing app configurations and rejects duplicate canonical configuration paths and managed names, domains, or image repositories. Application and update hostnames share one case-insensitive namespace, so an app cannot take another app's update hostname.

`lk workspace explain|check` starts no Cargo process, plugin program, database, provider operation, or release. Output is an allowlist of app configuration fields with explicit `not_checked` evidence states. The command does not serialize the complete app configuration or read secret environment values. Physical database ownership cannot be inferred from database declarations.

## Acceptance and remaining integration

The configuration tests cover readiness cycles, missing references, duplicate dependencies, bidirectional calls, binding variable conflicts, versions, paths, and deterministic ordering. CLI tests invoke the real binary without tools on PATH and check that secrets and database URLs are not consumed, that no build state appears, and that duplicate or escaping app paths fail. Schema tests cover structural editor validation; cross-reference and filesystem rules remain runtime checks.

Before publication, review generated help and schema assets, run the normal full gate on current main, and execute the two-application acceptance described below. The focused runs do not establish provider or container deployment acceptance.

The current configuration crate, including TOML deserialization and service configuration, was compiled directly from its unchanged production source against existing cached dependencies. The exact workspace inspector source and its integration tests ran through a small CLI dispatch harness: ten tests passed with no skips. This covers real app configuration loading, symbolic-link escapes, duplicate configuration paths, hostname/image/name collisions, bounded manifest input, and nested invocation with no tools on PATH. The same tests against the prior inspector failed on hostname collision as required. The harness does not test the complete CLI dispatch, plugin interception, or help snapshots.

The actual schema generator was also compiled from production source with the `schema` feature. It produced the new workspace asset and passed `--check`; the three existing application assets stayed byte-identical. All five production schema tests passed, including real example configurations and published aliases. These direct-source runs avoid rebuilding dependencies while another task owns the machine's full build slot; they do not replace Cargo feature-resolution, Clippy, or the normal pre-push gate.

A separate Rust probe compiled the exact graph implementation after removing only the serialization imports, attributes, and derive macros. It checked all 4,096 directed four-node graphs against an independent permutation oracle, plus bidirectional calls and invalid references, variables, versions, and paths: four tests passed with none skipped. This establishes the pure graph behavior only. It does not validate TOML decoding, generated schemas, filesystem inspection, CLI integration, or deployment.

## Pinned release execution

`workspace plan` reads a bounded TOML release set, validates the selected applications and all of their readiness dependencies, and binds the exact workspace bytes, parsed app configurations, versions, independently pinned A04 descriptors, and operator environment-variable references. Each descriptor must match its app and migration history. Plans cannot overwrite existing files. Values of referenced credentials are never serialized.

`workspace deploy` requires the independently reviewed plan hash. Before running any per-app command, it revalidates every selected app, resolves all declared environment references and stages exact descriptor bytes in a private directory. A workspace file lock prevents concurrent orchestration from that checkout. The existing per-app local and remote reservations remain authoritative. Every per-app invocation checks its configuration hash again before infrastructure operations.

Execution follows readiness order with a bounded local process-tree deadline. Progress is atomically persisted and synced; failure stops the remaining apps. Earlier apps retain their new releases. A timeout can leave a remote reservation and requires explicit status/recovery inspection. This is not a cross-app transaction.

Retry always invokes every selected app. The new `deploy --reuse-current` path only accepts the latest completed release with matching recorded configuration and descriptor. It rechecks current approvals and policy, image contents, database history, running image digest, effective app environment and HTTP readiness. Approval policy is checked again after observations. Legacy history without configuration evidence and incomplete operation phases require a new version or explicit recovery. Recording the complete phase now follows backup installation and pruning so a cleanup failure cannot look reusable.

The readiness probe requires exactly one expected running web container on each selected host, checks declared environment values without printing them, and requires an HTTP 200 from `/readyz`. SSH and Docker observations remain operator-host trust boundaries; they are not runtime attestation or proof of proxy traffic routing.

Focused Cargo acceptance initially passed 71 tests, covering the existing infrastructure command suite, actual workspace inspection, staged-input mutation, per-app command arguments, durable partial-failure retries, environment controls and deadline cleanup. The readiness script additionally ran seven local HTTP/Python cases, including Python optimized mode, stopped/replaced containers, changed credentials and redirects/errors. Docker and provider boundaries in these command tests are fixtures. A final 67-case focused run passed after adding legacy-history and before-infrastructure configuration checks. The approval-policy command case also exercises the new current-release reuse path. CLI Clippy passed with all features and targets, and the normal schema generation task produced no further asset changes. Help proposals were reviewed individually before acceptance. Full validation and real two-app deployment remain required before publication.

The rest of A02/X04 remains required:

1. Resolve each app through its actual Cargo package/feature graph. Distinguish a build dependency from a call or readiness dependency.
2. Build separate artifacts from declared source/recipe identities and establish cross-app API/schema compatibility before traffic changes. The current plan consumes prebuilt A04 descriptors and preserves their explicit unsupported/not-checked states.
3. Compute change impact from source/dependency and artifact identities. Current-release reuse avoids restarting an already selected and freshly revalidated app, but does not infer which apps a source change affects.
4. Resolve private endpoints and caller-specific credentials through the existing service identity work. Bindings alone never authorize calls or forward user identity.
5. Establish database ownership and online schema compatibility windows across apps. Readiness ordering does not establish either property.
6. Prove two applications deploy separately, one changes without rebuilding/restarting the other, an external generated client works, and incompatible API/schema changes fail before traffic changes. Include interruption, stopped dependency and rotated credentials in the real acceptance fixture.

PR553 and the existing runtime, database, identity, and acceptance stacks are separately owned. Inspect their latest rebased heads before integration; do not import the same service base repeatedly from older child branches.
