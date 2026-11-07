---
"leptoskit_config": minor
"leptoskit_cli": minor
---

# Describe independently configured applications

Add a separate `leptoskit.workspace.toml` manifest and `lk workspace explain` / `check`. Each application references its own configuration. Startup dependencies are validated independently of call bindings, so bidirectional calls do not become readiness cycles. Unknown references, escaping paths, repeated configurations, conflicting managed identities, and duplicate binding variables fail before a build starts.

The versioned report exposes only selected configuration and environment variable names. It explicitly marks artifacts, API compatibility, database ownership, service identity, and deployment as not checked. Workspace inspection runs no plugin programs or build tools. Image creation, binding injection, change impact, and release orchestration remain future consumers of this graph.
