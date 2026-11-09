# Graph provisioning plans

Graph declarations previously generated provider resources but rejected `infra plan`, `apply` and `output` through the managed application settings path. These commands now have graph dispatch before managed settings are read. Managed deployment and verified release packaging retain their existing paths.

## Saved-plan contract

The graph operation takes an exclusive lock in the owner-only `infra/<name>` directory. A new plan invalidates any previous receipt before invoking OpenTofu. The generator also invalidates the receipt before rewriting inputs. Generated provider, graph report and per-host edge bytes must reproduce exactly from the current declarations and retained Cloudflare CIDR snapshot. Applying never silently refreshes that snapshot.

The receipt records SHA-256 hashes of the full serialized configuration, generated files, provider lock and binary saved plan, plus backend metadata/workspace-file presence and hashes. It also records the reported OpenTofu version/platform and default workspace. It accepts stable OpenTofu 1.10 or later within major version 1. An operator environment uses its declared name and backend key rather than an implicit Terraform workspace. CLI argument, data directory and workspace environment overrides are rejected. Provider credentials and declared origin CSR input remain operator inputs; the saved binary plan retains evaluated plan variables.

The provider lock is bounded to 1 MiB, the saved plan/backend metadata to the shared 16 MiB metadata limit, and captured tool JSON to 64 MiB. Relative file links are rejected by the shared descriptor-based file reader. Generation and apply are coordinated against other CLI graph operations; the operator account, OpenTofu, installed providers, backend and machine remain trust boundaries. This is not a signature, independent approval, source reproduction or protection against an operator replacing both inputs and their local receipt.

Plan inspection refuses deletion/replacement of declared cloud host and network resource types, including when their resource blocks disappear. Unknown action kinds and incomplete/errored plans fail. A new explicit environment is required for host migration before retirement. The aggregate compute guard obtains current prices for each used provider before planning and applying; currencies are never converted implicitly. This is not a total billing cap.

Apply rechecks configuration, files, engine identity, workspace and plan actions, then consumes and synchronizes the receipt before invoking OpenTofu. A failed or interrupted apply requires a fresh plan. Backend state locking remains enabled. There is no distributed operator approval protocol in this receipt.

## Output boundary

Only `tofu output -json hosts` and `tofu output -json domains_by_host` are read. Host names, providers, locations, sizes and domains must match the graph. Malformed, unexpected or non-public address values fail; parse diagnostics omit raw provider values. Output identifies provider state as observed and host reachability, application runtime and attestation as unchecked or absent. Tunnel tokens are not requested.

## Acceptance and remaining work

Production module tests cover altered generated files, recipes, provider lock and saved plan; backend identity changes; failed replanning and apply; engine/workspace drift; invalid plan actions; budget failures; extra Terraform inputs; mid-operation mutation; relative links and lock contention; public output placement/domain validation and sensitive error suppression. Pricing tests cover aggregate hosts, missing offers and mixed currencies.

`tests/graph_infra.rs` exercises actual CLI dispatch with a fake engine and no Cargo project. It covers the ordinary lifecycle, public output, substitution, failed replan, implicit argument rejection and failed apply. Full CLI execution and its normal push gate remain required before publication.

The ignored `infra::graph::lifecycle::acceptance::real_local_saved_plan_inspection_and_apply` runs the production process adapter and inspection against a real OpenTofu executable on PATH. It creates only the built-in local `terraform_data` resource, applies a saved plan, reads a named public output, and checks stale/corrupt plan rejection. It does not exercise real cloud resources or replace the graph provider-schema fixture. The scoped harness copies production graph/provider/edge/lifecycle/pricing and asset code unchanged, excluding graph build capture and substituting only the enclosing command enum. It does not establish full CLI integration.

Application artifact installation, edge binary and certificate installation, tunnel supervision, readiness, release switching, rollback, origin ownership and graph release authorization remain open A02-WEB work. A successful provisioning apply must not be described as a deployed or verified application.

The parent scoped run passed 46 production tests and workspace Clippy rules. Three integration fixtures were ignored in that ordinary run; the new real OpenTofu fixture was then explicitly run and passed on 1.13.9 for Darwin ARM64. The existing Caddy and mocked-provider fixtures belong to the separately validated graph foundation. Removing the saved-input comparisons in an isolated source copy made the changed-provider-file rejection test fail as required. The production source retained those comparisons.

The new lifecycle modules pass the repository's monostyle threshold. Whole-tree monostyle reports 79.9 readability on both this worktree and an untouched export of its parent commit, below the required 80; this is not reported as a passing full gate. Full CLI command tests, current-main integration and the normal push gate remain pending. No cloud provider was contacted by the local saved-plan acceptance.
