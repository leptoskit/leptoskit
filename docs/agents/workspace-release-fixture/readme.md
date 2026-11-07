# Local workspace release fixture

This dependency-free backend exports an empty RPC specification and serves `/readyz`. It is intended only for local release orchestration acceptance. It does not implement application authentication or database migrations. The acceptance driver must create and inspect each real Postgres migration table separately; an HTTP 200 alone does not prove database readiness or cross-application compatibility.

Build and package the committed fixture tree through the verified-build commands, using independently pinned builder and runtime images. Use distinct app configs, image repositories and managed names for the two deployments. Keep provider state local: this fixture does not establish cloud provisioning acceptance.
