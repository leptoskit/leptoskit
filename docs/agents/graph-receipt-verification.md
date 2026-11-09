# Graph receipt consumption

Graph captures are ordinary local build outputs. Their manifests record integrity and declarations; they do not prove source reproduction, publisher identity, release approval, a Linux executable target or remote runtime behavior.

`verify::graph::check` opens a receipt directory, reads at most 16 MiB of metadata without following links, compares exact manifest bytes to an independently supplied SHA-256, then validates the typed manifest. Duplicate object keys are rejected at every level, including escaped aliases and asset paths. The adjacent `manifest.sha256` is checked for consistency but never supplies the independent trust anchor.

The consumer requires the current configuration digest, exact selected recipe hashes, declared host placement, frontend/service artifact shape and readiness dependencies. Static frontends require a nonempty index; SSR may have an empty public tree but requires a separate executable. Frontend ownership exclusions and private backend paths cannot reappear in public assets. Service runtime image pins are read from the current project and checked again during consumption. Ordinary receipt fields cannot upgrade `not_checked` evidence into verified provenance.

Exact file checks retain directory handles, reject links/special files, compare complete membership, and verify modes, byte counts and hashes. The aggregate receipt budget is 100,000 artifact entries and 16 GiB, with the existing per-tree depth limit. Capture calls the same consumer before publishing success so it cannot produce receipts that exceed the reader's contract.

## Selected staging

The CLI `lk verify graph` runs inside the graph project. With `--out`, it creates a new child under an existing parent; it never overwrites an output. The child is created with Unix mode 0700 before files are copied. The first runtime test exposed the default temporary-directory mode as 0755, so capture and staging now request the mode explicitly.

Each selected unit retains its exact public tree and/or executable. The stage also contains the original `source-manifest.json`, a `stage.json` naming the exact source digest, selected units and readiness-only dependencies, and a convenience checksum. Nonselected units are not copied or restarted. A readiness declaration is not a successful readiness probe. Inputs are rechecked before and after copying; failure removes the temporary child.

The checked handle has a private constructor. It is an in-process validation result, not a serialized token that may bypass checks. Staging does not execute a binary, build an image, invoke a provider, provision a host, authorize a release or activate traffic. Future packaging and deployment consumers must revalidate their own inputs and policies rather than treating `stage.json` as authorization.

## Acceptance and remaining integration

The scoped harness compiles the production receipt, filesystem capture and build-capture modules against existing cached dependencies, using the pinned compiler and repository Clippy policy. Tests execute actual shell recipes, check static/SSR/service captures, exercise original-pin substitution, duplicated keys, malformed declarations, membership/mode changes, owner exclusions, changed toolchain pins, aggregate budgets, readiness-only selections and private staging. One extra harness test only links the CLI entry point and is excluded from production test counts.

`tests/graph_build.rs` also contains the full CLI build-to-check-to-stage journey and requires rejection after artifact replacement without rerunning recipes. Full CLI compilation/execution, reviewed help snapshots, normal push hooks and rebase onto the active graph foundation remain required. The source changes in `infra/graph/build/capture.rs` and `verify/assets` must preserve the foundation delegate's acceptance fixes. The earlier edge slice remains a prerequisite.
