# Several targets in one application

A product can have a public website, a private UI, a relay server and native apps without becoming several Leptoskit applications. Keep one root `leptoskit.toml`, one Cargo workspace and one lockfile. Each frontend names its UI package, routes and public inputs; each target selects a frontend and a platform. A server-only target selects a server package directly.

```toml
[app]
name = "journal"
default_target = "desktop"
default_platform = "macos"
platforms = ["web", "macos", "server"]

[frontends.product]
package = "journal_app"

[frontends.public]
package = "journal_site"
server_package = "journal_site_server"
render = "ssr"

[frontends.public.inputs]
web = "web/public"
public = "public/site"
assets = "assets/site"

[frontends.public.router]
dir = "crates/journal_site/src/routes"

[targets.site]
frontend = "public"
platform = "web"

[targets.desktop]
frontend = "product"
platform = "macos"

[targets.desktop.native]
mode = "menu-bar"
package_id = "com.example.journal"

[targets.relay]
platform = "server"

[targets.relay.build]
backend = true
server_package = "journal_relay"
```

From the root, run `leptoskit targets` to validate and list destinations. `leptoskit --app-target site dev` serves the public frontend; `leptoskit --app-target relay build --release` builds the relay. The selector applies to nested commands too. `LEPTOSKIT_APP_TARGET` provides the same selection to build subprocesses. No selector uses `app.default_target` when present; applications without named targets keep their existing behavior.

All paths stay relative to the root configuration, including file routes, frontend inputs and database migrations. Default output is `dist/<target>`; an explicit `--out` takes precedence. The selected server and sidecar build records also stay separate, so serving one target cannot launch another target's server. Application identity and native package IDs do not have to match the server's Cargo package name.

Targets can select `serve`, `db`, `infra`, `native` and `build` tables. Supplied tables replace their application-level counterpart as a unit. A frontend can set its own `lint` table and tooling `plugins`. Databases and infrastructure are specific to each named target. `services = ["search"]` selects entries from the existing application-wide services registry. An omitted or empty services list starts no sidecars, so a public site cannot implicitly start the product's private services. See [Services](32-services.md).

The macOS `menu-bar` mode creates a native status item without opening a window or starting its Wasm interpreter. Choosing Open creates the native UI; closing it releases the interpreter while the status item remains. Neither desktop mode requires a URL or a web server. A custom app-owned Swift host keeps its own lifecycle. Native runner crates can live in `platforms/macos`, `platforms/ios` or `platforms/android`; the existing platform and `hosts/` layouts continue to work. Generated native state stays in `.leptoskit`.

Declaring a target does not implement missing platform capabilities. Put an explanation in `targets.<name>.unavailable` while a destination is unfinished: selection fails before building. HarmonyOS remains experimental; mobile capture and background permissions must be implemented by the application and its platform plugins.

Use [applications in one workspace](33-workspaces.md) when the directories represent independent applications with their own configurations and release identities.
