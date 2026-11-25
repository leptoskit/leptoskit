# Shared LeptosKit overview and cloud blocks

Edit these blocks, then run `devenv shell docs:update` (or `mdt update`).
Keep relative file links outside these blocks so consumers in `readme.md`,
`docs/src/`, and `skills/` all keep valid links.

<!-- {@leptoskitOverview} -->
leptoskit is a full-stack Rust toolkit for taking an app from a single codebase to every screen and to production:

- **One UI, real native widgets:** describe screens once as a tree of components (`View`, `Text`, `Button`, `TextInput`, `ScrollView`, …) styled with built-in Tailwind. On the web it renders to the DOM with SSR, CSR, or islands; on macOS, iOS, Android, Linux, Windows, the terminal, browser extensions, and experimental HarmonyOS it renders native controls with no web view and built-in signed WebAssembly over-the-air updates.
- **Full-stack by default:** typed server functions (`leptoskit_rpc`), file-based routing, cached queries, Rust-struct SQLite or Postgres schemas and migrations (`leptoskit_db`), passkey/OAuth sign-in and OIDC IdP (`leptoskit_auth`, `leptoskit_idp`), background jobs, real-time updates, and native device plugins live beside the UI in one workspace configured by `leptoskit.toml`.
- **Built for AI agents:** compiler-checked builder syntax by default (`[lint] markup = "builder"`), a built-in MCP server (`lk mcp`) for live DevTools inspection and cloud deploys, safe runtime generative UI (`leptoskit_agent_ui`), verifiable releases (`leptoskit verify`), a first-party skill, and `llms.txt` docs.
- **Two clear ways to ship:** deploy straight from `leptoskit.toml` to **LeptosKit Cloud** (`lk cloud launch` / `lk deploy`), or self-host on your own Linux VMs with **OpenTofu + Kamal** (`lk infra`).
<!-- {/leptoskitOverview} -->

<!-- {@deploymentTracks} -->
| Track | Best for | Primary commands | What it manages |
| --- | --- | --- | --- |
| **LeptosKit Cloud** (managed) | Shipping fast with zero infrastructure boilerplate, including from AI agents | `lk cloud login`, `lk cloud launch`, `lk deploy`, `lk cloud status`, `lk cloud logs` | Source archiving, OCI builds, rolling releases, scale-to-zero, managed Postgres, secrets vault, custom domains, and usage metering from `[cloud]` in `leptoskit.toml` |
| **Self-hosted** (`lk infra`) | Running on your own Hetzner, DigitalOcean, or Linux servers with full provider control | `lk infra generate`, `lk infra plan`, `lk infra apply`, `lk deploy` | OpenTofu VMs/networks/Cloudflare tunnels, Kamal app + Postgres containers, and encrypted Restic backups (`infra.mode = "graph"` is experimental) |

`lk console` is the legacy operator-enrolled deployment dashboard and agent API; new projects should pick either **LeptosKit Cloud** (`lk cloud`) or **Self-hosted** (`lk infra`).
<!-- {/deploymentTracks} -->

<!-- {@cloudQuickstart} -->
```sh
# 1. Create a cloud-ready app (add --web-only to skip mobile/desktop runners)
lk new notes --cloud --auth --web-only
cd notes

# 2. Sign in to LeptosKit Cloud in your browser
lk cloud login

# 3. Create & link the project, preview monthly cost, and ship the first release
lk cloud launch

# 4. Ship subsequent changes, check readiness, and stream logs
lk deploy
lk cloud status
lk cloud logs --follow
```
<!-- {/cloudQuickstart} -->

<!-- {@cloudLimits} -->
- **Control plane & cluster packaging:** the control plane (`leptoskit-cloud-control`), cluster/build workers (`leptoskit-cloud-worker` with its built-in `build-app` OCI builder), Dockerfiles, GHCR multi-arch workflow, Kubernetes manifests, and the `leptoskit-cloud-platform up` operator orchestrator are implemented and covered by unit, HTTP, Postgres, and manifest snapshot tests.
- **Live cluster validation:** end-to-end provisioning on live Hetzner, Cloudflare, S3/R2, and Stripe accounts is still gated on operator acceptance runs (live gVisor isolation, sleep/wake traffic, WAL backup/restore, and Stripe checkout verification).
- **Databases & storage:** shared managed Postgres (`size = "xs"`) is provisioned automatically per environment; dedicated database sizes (`s`, `m`, `l`), automated cluster-worker dump/restore, and automatic bucket provisioning for `[cloud.storage.buckets.<name>]` are still in progress.
<!-- {/cloudLimits} -->
