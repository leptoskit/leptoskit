# Auth generation in Cargo workspaces

The auth generator used to look for literal workspace member paths and a server at the app's `src/main.rs`. Deployment already used Cargo metadata. Consequently a project could deploy successfully but fail to generate sign-in screens after moving its configuration or packages into a nested workspace.

`auth_pages::layout` now reads `cargo metadata --no-deps` before writing. It resolves an explicit client package, otherwise the app's client library or a unique client candidate, with a single-package fallback. Backend selection keeps the package-named or single binary rule used by deployment. An explicit `build.backend = false` suppresses server generation; an explicit backend enables the server feature on a separate server package. Inherited dependency features participate in the existing automatic server detection.

The generator writes modules beside Cargo's actual library and binary source targets. It computes both the embedded configuration path and its configuration base relative to the selected app directory. Canonical manifests and target sources must remain inside the Cargo workspace. Invalid or ambiguous target selection and missing or renamed toolkit dependencies are rejected before any generation writes. Custom application code is retained under the existing ownership rules.

This does not automatically retrofit account ownership into existing tables, choose authorization policies, mount the auth layer, or wrap arbitrary application roots. Those integration steps remain explicit. The generated template still expects the dependency name `leptoskit`; renamed dependencies receive a diagnostic before writes.

## Local evidence

The scoped harness compiles the production layout module and runs real Cargo metadata against temporary workspaces, with no build or dependency resolution in the fixtures. Its seven resolver tests cover globbed members, inherited manifests, custom targets, nested app directories, automatic client selection, static mode, ambiguity, missing/renamed dependencies, a source symlink outside the workspace, library-only apps, and module-name collisions.

The final combined harness also compiles the production auth generator and configuration crate. It uses the actual router-declaration function and scaffold plugin constants; the route-generation boundary deliberately panics if called. Four router tests are excluded, and the unrelated native-class test is disabled because that renderer is outside this harness. The remaining tests exercise real file generation, configuration edits, idempotence, preservation, and failure behavior. This is not a full CLI or generated-app compilation claim.

The old production generator is also compiled in a separate negative-control harness. The new nested-workspace and ambiguous-server/no-write tests must fail against it. Logs, exact harness sources and results are retained under `/Users/mini03/Developer/leptoskit-audit-state-20261107/auth-workspace-evidence`.

Before publication, run the complete `auth_pages` test suite and scaffold snapshots in the CLI, compile a generated nested app with its server's embedded configuration, and run the full normal local gate and push hook. No native device or cloud deployment is claimed by the scoped harnesses.
