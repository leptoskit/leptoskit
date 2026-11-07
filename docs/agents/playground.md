# Playground implementation and deployment requirements

The `/play/` shell is rendered during site packaging with the starter source in its textarea. Its JavaScript controller loads only on this page. The landing page receives two link changes. `LEPTOSKIT_PLAY_SERVICE` sets the service origin at build time (default `https://play-api.leptoskit.dev`).

## Compile boundary

The Axum router resolves the configured toolchain image to an immutable image ID at startup. Each request starts a fresh container. Source goes over stdin; no host source directory or runtime socket is mounted into that container. Compiler output is returned as JSON with JavaScript, base64 Wasm, raw diagnostic JSON, and text. Diagnostics that point into the fixed harness remain in `raw_diagnostics`; only real locations in `user.rs` become editor links. Only metadata enters server logs. The service caches successful responses by image ID and SHA-256 of the exact source. Prewarmed starters remain cached when ordinary edits fill the cache; both count against the configured entry limit. The bounded in-memory cache is lost on restart; there is no database.

The toolchain image precompiles the fixed Cargo harness. The worker invokes rustc against those immutable dependencies directly, compiling only `user.rs` and the mount wrapper. This avoids copying a large Cargo target directory into each request's scratch filesystem. No Cargo manifests, build scripts, runtime arguments, dependency versions, or mounts come from the request. Readers can use real Rust and the toolkit's `view!` macro. The function contract is `pub fn app() -> impl Fn() -> Element`; allocating signals outside that closure preserves state across reactive renders.

The image and production service Dockerfiles both build from the repository root. Build the toolchain in the same runtime the service uses: `DOCKER_HOST=unix:///var/run/docker.sock CONTAINER_HOST=unix:///var/run/docker.sock CARGO_BUILD_JOBS=4 devenv shell -- docker build -f crates/leptoskit_playground_server/image/Dockerfile -t leptoskit-playground-toolchain:local .`. `wasm-bindgen-cli` is pinned to Cargo.lock's `0.2.129`. The measured latency-oriented profile uses opt-level 0, no debug information, 256 codegen units, no incremental cache, and no wasm-opt. Cargo's artifact manifest supplies dependency paths, including both `.rlib` and `.rmeta` files; do not assume a flat `target/debug/deps` layout.

To add a crate, add it to the harness manifest, rebuild its dependencies, and add its name to the explicit `--extern` list in `worker.rs`. Review proc macros and build scripts as trusted image-build inputs. Never permit request-provided Cargo.toml files. Release image construction must preserve the matching repository lockfile and publish an immutable digest.

The trusted base includes the toolchain, precompiled dependencies, container runtime, and Linux kernel. Containers share that kernel, so a compiler or runtime escape remains a risk: use a dedicated, patched host and a dedicated runtime account. The build context excludes dotenv credentials and local agent/build caches. Compile containers receive no host environment or runtime socket.

## Limits

All configuration comes from environment variables:

| Variable                     | Default                                                    |
| ---------------------------- | ---------------------------------------------------------- |
| `PLAY_BIND`                  | `127.0.0.1:4702`                                           |
| `PLAY_IMAGE`                 | `leptoskit-playground-toolchain:local`                     |
| `PLAY_SOCKET`                | `/var/run/docker.sock`                                     |
| `PLAY_RUNTIME`               | `docker`                                                   |
| `PLAY_ORIGINS`               | production site plus localhost / loopback port 4701        |
| `PLAY_WORKERS`, `PLAY_QUEUE` | available CPU cores each                                   |
| `PLAY_TIMEOUT_SECONDS`       | 30                                                         |
| `PLAY_MEMORY_MIB`            | 1024 per compile                                           |
| `PLAY_CPUS`                  | 1 per compile                                              |
| `PLAY_PIDS`                  | 128                                                        |
| `PLAY_SCRATCH_MIB`           | 256                                                        |
| `PLAY_REQUEST_BYTES`         | 65536 including JSON                                       |
| `PLAY_OUTPUT_BYTES`          | 16777216 per output pipe                                   |
| `PLAY_CACHE_ENTRIES`         | 32                                                         |
| `PLAY_REQUESTS_PER_MINUTE`   | 12 per transport peer                                      |
| `PLAY_PREWARM`               | true; compile and cache the five starters before readiness |

Containers use `--network none`, read-only root, UID 65534, all capabilities dropped, no-new-privileges, no swap beyond their memory limit, and bounded tmpfs mounts. `--log-driver=none` keeps diagnostics and compiled code out of the container daemon's journals; attached output streams go to the requesting service only. An internal GNU timeout supervisor kills compilation even if the service process disappears. The host also forcibly removes the container after completion, timeout, or output overflow. A bounded admission semaphore returns 503 and Retry-After when all running and queued slots are occupied. Request and output limits protect host memory as well as compiler memory. The rate-limit address table has a fixed cardinality cap.

`PLAY_CLOUDFLARE_CIDRS` is a comma-separated list of trusted Cloudflare IPv4/IPv6 CIDRs. It defaults to empty: forwarded IP headers are ignored until the operator installs the provider's current published ranges. Only matching transport peers may supply CF-Connecting-IP. Keep these ranges current together with the firewall. Never trust arbitrary reverse proxies or X-Forwarded-For.

The production service image sets `PLAY_ORIGINS=https://leptoskit.dev`. The native development defaults also permit localhost on port 4701. Ingress must preserve the transport peer's address for Cloudflare recognition; validate this through the chosen rootless networking mode before deployment.

## Browser boundary

The iframe has `sandbox="allow-scripts"` and deliberately lacks allow-same-origin. It receives the bundle through postMessage; the parent validates the sending window and opaque origin. The frame CSP permits only its controller, blob modules, Wasm compilation, inline generated styles, and local images. It denies connections, forms, objects, and other network resources. Wasm is initialized from bytes, never a fetched URL. Compiler output and diagnostics enter the DOM through textContent.

The frame sends heartbeats; the parent offers Stop and Reload when it misses them. The real endless-loop test passes in headless Chrome, whose sandboxed frame runs in a separate renderer target. **This is not a portable CPU isolation guarantee:** another browser may schedule an opaque iframe in the parent's renderer process. A synchronous infinite loop can then prevent both watchdog and Stop from running. Safari and Firefox recovery remain unverified.

The broker serves `/play/frame.html` with its own CSP and `/play/frame.mjs` with anonymous CORS. Static hosting must install equivalent per-path headers; a meta policy can tighten but cannot relax the broker's global policy. The `/play/` document also needs connect-src permission for the compile API. A policy that only allows default-src self will block the service request. Keep blob execution permission confined to the preview document.

## Future lk infra service declaration

Do not provision this service separately from the toolkit's service graph. The future declaration needs an always-warm Linux container host, an immutable production service image, access to a dedicated container runtime socket, `play-api.leptoskit.dev`, `/healthz` readiness, environment limits above, structured stderr collection, and no database. The socket is effectively host authority; do not share it with unrelated workloads. Prefer rootless Podman under a dedicated account after validating resource limits and cleanup against its Docker-compatible API. Rootless memory/pid limits require delegated cgroup v2 controllers. Fail deployment if limits cannot be enforced.

The production service image uses Debian's `docker-cli` package; `docker.io` alone supplies the daemon without that executable. A local image smoke test compiled the real counter through Docker CLI against rootless Podman. On the SELinux-enabled test VM, the service container needed `--security-opt label=disable` to access its explicitly mounted runtime socket. Apply equivalent socket access deliberately to the trusted service only; compile containers retain their normal security labeling and never receive the socket. Validate UID mapping and socket permissions in the eventual host configuration.

The firewall must admit API traffic only from Cloudflare's published IPv4 and IPv6 ranges, with a separately controlled administration path. CORS is browser policy, not authentication or protection against direct HTTP abuse.

## Measurements and sizing

Measured on 2026-11-07 against the Docker-compatible Unix socket of an arm64, rootless Podman VM with six CPUs and 8,289,566,720 bytes of memory. The local `docker` executable is a Podman wrapper: the service sets both DOCKER_HOST and CONTAINER_HOST so the configured socket also selects its connection. The host was busy (load average about 324 during initial preflight). These numbers describe this local run, not a Hetzner performance promise.

After the service prewarmed all five starters, ten distinct visible edits to the counter, each in a new 1-CPU/1-GiB container, took 3.894, 3.856, 3.657, 4.400, 4.062, 3.801, 3.850, 4.858, 4.123, and 5.054 seconds. Nearest-rank p50 is 3.894 seconds and p95 is 5.054 seconds. The native host binary uses the production release profile; the guest uses the latency-oriented profile above. Timing includes container lifecycle, rustc, bindgen, artifact serialization, and cleanup, bypassing the HTTP result cache. Outside the timed interval, the measurement decodes each Wasm bundle and verifies that every edit produced a different artifact hash. Run `cargo run --release -p leptoskit_playground_server --bin playground-measure -- crates/leptoskit_app/web/play-counter.rs` after prewarming the same image to repeat it.

An earlier debug-host batch included host-side Wasm decoding and hashing in its timer and reached p95 11.583 seconds. Investigation measured container startup/exit at 0.37 seconds, the worker at 1.814 seconds, and forced cleanup at 0.093–0.276 seconds. Instrumented runtime phases took 1.806–2.873 seconds. These observations motivated correcting the timer and using the production host profile, rather than changing guest optimization. Dependencies are already compiled; sccache would miss distinct sources, wasm-opt would add work, and keeping containers alive would add isolation complexity to save only the small lifecycle cost. Different host load conditions also affect these batches; this is not a controlled speedup comparison.

The edited counter bundle is 6,880,097 bytes of Wasm plus 42,632 bytes of JavaScript before transfer compression (the unedited starter's Wasm is 6,880,089 bytes). The final rootless `docker build --no-cache` took 634.13 seconds including the base pull, producing an arm64 image of 3,302,751,631 bytes with ID `sha256:f650c9f464c497f5fea770615c4cd9167da85760288f4045766006b090cf75f9`. Integration tests and final latency measurements use that image directly. A separate cgroup probe recorded 213,573,632 bytes peak memory and 1,601,348 microseconds total CPU, with memory.max=1,073,741,824, pids.max=128, and cpu.max=`100000 100000`. The memory probe is one compile, not a worst-case bound; the 1-GiB cap remains necessary for arbitrary input.

Start planning around four CPU cores and 8 GiB RAM, with four workers and four queued requests: up to 4 GiB of compile cgroups, up to 512 MiB of cached responses, and headroom for the service, runtime, and OS. A provisional Hetzner choice is CPX32 (4 vCPU / 8 GB), budgeting approximately €36/month before VAT, IPv4, and backups. The indexed official June 2026 price notice lists €35.49; its live URL returned 404 during verification, so confirm the actual quote in the [Hetzner calculator](https://www.hetzner.com/cloud/) before using it in infra defaults. Re-run these measurements on its x86 hardware. CAX21 has the matching arm64 shape but the public product page currently marks it unavailable.

The release workflow publishes the toolchain to GHCR and retains the immutable registry reference in its `playground-toolchain-pin` artifact. A future service declaration must pull that reference into the dedicated runtime account and set PLAY_IMAGE to it before startup. Do not mount the runtime socket into compilation containers. No infrastructure has been provisioned or deployed.
