---
"release": minor
---

# Explicit deployment graph ownership

Add a shared graph model for independently released frontends and services. It checks route and artifact ownership, exclusive host placement, declared container-runtime requirements, and readiness dependencies. A release selection builds shared artifact inputs once while keeping unrelated release units out of the deployment selection.

Path mounts use segment boundaries, retain missing assets with their owning frontend, and redirect bare mount paths to their trailing-slash form. Frontends can partition one build output into separate release trees without publishing the same files twice.

Introduce explicit `infra.mode = "graph"` declarations with named artifacts, static/SSR frontends, per-service placement, readiness and rootless runtime requirements. Static projects do not inherit the managed preset's Postgres or backup settings. Ports are checked per host; ambiguous output paths, overlapping build outputs and conflicting bindings are rejected.

`lk infra graph --unit frontends.docs` inspects ownership and the exact release selection without building or contacting providers. `lk infra generate` writes per-host Hetzner/DigitalOcean resources and proxied Cloudflare DNS, with distinct HTTPS and Tunnel ingress. HTTPS origin rules use a recorded dual-stack Cloudflare range snapshot; static-only graphs add no database or backup resources. Artifact, provider and runtime evidence remains explicitly unchecked. Graph plan, apply and release remain unavailable until their lifecycle adapters and acceptance land.

Existing TOML retains its managed behavior and serialized shape. Rust callers constructing infrastructure directly now use `ManagedInfraConfig` and convert it into the `InfraConfig` lifecycle enum. Graph persistence, external service nodes, static publication and deployment remain follow-up work.

Single-app dev, serve, native and deployment commands refuse graph roots before selecting default targets or generating app files. Graph builds select qualified units with `lk build --unit frontends.docs`, run shared recipes once, and capture disjoint asset trees and executables with exact manifests. Service builds keep their Cargo features separate. The website packager accepts the invoking CLI and graph profile. These local build receipts do not claim isolated source reproduction or runtime deployment.
