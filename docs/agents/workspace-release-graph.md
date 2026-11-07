# Workspace release graph

The A02/X04 workspace layer references complete application configurations. It complements PR553's processes inside one app; it does not replace that app's service registry or pretend that Kamal roles sharing one image are independent artifacts.

The public contract is `leptoskit_config::workspace::Workspace`. `applications.<id>.path` resolves beneath the manifest root to an existing `leptoskit.toml`. `depends-on` is a readiness graph; `bindings` is a separate directed call graph and may contain cycles. Graph validation is deterministic. The CLI validates existing app configurations and rejects duplicate canonical configuration paths and managed names, domains, or image repositories.

`lk workspace explain|check` starts no Cargo process, plugin program, database, provider operation, or release. Output is an allowlist of app configuration fields with explicit `not_checked` evidence states. The command does not serialize the complete app configuration or read secret environment values. Physical database ownership cannot be inferred from database declarations.

## Acceptance and remaining integration

The configuration tests cover readiness cycles, missing references, duplicate dependencies, bidirectional calls, binding variable conflicts, versions, paths, and deterministic ordering. CLI tests invoke the real binary without tools on PATH and check that secrets and database URLs are not consumed, that no build state appears, and that duplicate or escaping app paths fail. Schema tests cover structural editor validation; cross-reference and filesystem rules remain runtime checks.

Before publication, run schema:update, review the new workspace schema and existing alias stability, regenerate and review help snapshots, and execute config/CLI tests plus the normal full gate. The initial source implementation has not yet passed those runtime gates.

A separate Rust probe compiled the exact graph implementation after removing only the serialization imports, attributes, and derive macros. It checked all 4,096 directed four-node graphs against an independent permutation oracle, plus bidirectional calls and invalid references, variables, versions, and paths: four tests passed with none skipped. This establishes the pure graph behavior only. It does not validate TOML decoding, generated schemas, filesystem inspection, CLI integration, or deployment.

The rest of A02/X04 remains required:

1. Resolve each app through its actual Cargo package/feature graph. Distinguish a build dependency from a call or readiness dependency.
2. Freeze separate source, recipe, API, schema, and artifact identities per app. Consume A04 descriptors and operator compatibility pins; preserve unsupported evidence states until tested.
3. Compute change impact from source/dependency and artifact identities. Reuse unchanged artifacts and do not restart unaffected consumers when compatibility is preserved.
4. Resolve private endpoints and caller-specific credentials through the existing service identity work. Bindings alone never authorize calls or forward user identity.
5. Apply readiness order with bounded waits, idempotent retry/recovery and per-app durable release history. Keep schema ownership and compatibility windows explicit. No cross-service transaction is implied.
6. Prove two applications deploy separately, one changes without rebuilding/restarting the other, an external generated client works, and incompatible API/schema changes fail before traffic changes.

PR553 and the existing runtime, database, identity, and acceptance stacks are separately owned. Inspect their latest rebased heads before integration; do not import the same service base repeatedly from older child branches.
