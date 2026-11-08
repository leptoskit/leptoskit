# Applications in one workspace

Keep several applications in the same repository while giving each its own configuration, build, and deploy. A `leptoskit.workspace.toml` file describes those boundaries and checks how the apps depend on one another before a build starts.

Use this when a **marketing site**, the **signed-in product**, and an **admin console** should ship as separate frontends (different domains, bundles, and release records), or when a website and an accounts API share a repo. Prefer separate frontends over stuffing landing and product into one route tree; same-deploy route groups stay an escape hatch for a thin splash on the product origin.

Inspection runs on the development machine and reads configuration for web, server, and native apps. The release commands below run on a managed deployment operator's machine and deploy already built server or supported website images. They do not build native hosts or publish code-push bundles.

## Multiple frontends

Scaffold landing, product, and admin in one go. The flag alone selects all three; list names to pick a subset (`landing`, `app`/`product`, `admin`):

```bash
leptoskit new shop --frontends
# same as:
leptoskit new shop --frontends landing,app,admin
```

That writes:

```text
shop/
├── leptoskit.workspace.toml
├── README.md
└── apps/
    ├── landing/    # web only — marketing (port 3000)
    ├── app/        # web + native hosts — product package `shop` (port 3001)
    └── admin/      # web only — admin console (port 3002)
```

Each `apps/<name>` is a full leptoskit app with its own `leptoskit.toml`, file-router routes (same defaults as `leptoskit new`), and Cargo workspace. Ports are stable per kind (order in `--frontends` does not matter). The product app under `apps/app` uses the workspace directory name as its Cargo package (`shop_app`, not `app_app`). Landing and admin omit native host crates. Build and develop from inside one app:

```bash
cd shop/apps/landing
leptoskit build
leptoskit dev
```

The workspace file names each application's role with optional `kind`:

```toml
format_version = 1

[applications.landing]
path = "apps/landing"
kind = "landing"

[applications.app]
path = "apps/app"
kind = "product"

[applications.admin]
path = "apps/admin"
kind = "admin"
```

`kind` is a hint for tooling and docs (`landing`, `product`, `admin`, `server`). It does not change runtime by itself; platforms still come from each app's `leptoskit.toml`. Cross-frontend links are ordinary URLs between deployments. Shared sign-in across frontends is not part of this scaffold.

## Website and API

```toml
format_version = 1

[applications.website]
path = "apps/website"
depends-on = ["accounts"]

[applications.website.bindings.accounts]
application = "accounts"
url-env = "ACCOUNTS_URL"
token-env = "ACCOUNTS_TOKEN"

[applications.accounts]
path = "apps/accounts"
```

Each `path` contains its own `leptoskit.toml`. Paths are relative to the workspace file and must remain inside that directory, including after resolving symbolic links. Application names are stable workspace identities; their Cargo packages can have different names.

Run:

```sh
lk workspace explain
lk workspace check --json
```

The command finds the nearest workspace file in the current directory or its parents, so it also works from inside either app. Use `--manifest path/to/leptoskit.workspace.toml` to name a file explicitly. `check` exits with an error for invalid app configuration, unknown dependencies, startup cycles, repeated app configuration paths, or conflicting managed infrastructure names, domains, and image repositories. Application and update hostnames must be distinct across apps, including differences only in letter case.

## Readiness and calls are different

`depends-on` means that a provider must be ready before its consumer starts. These edges must have no cycles. The report gives a deterministic startup order.

A binding describes a call from one app to another. `url-env` and `token-env` are variable names for that caller. The command never reads their values. Binding names and destination apps are checked, and two binding fields cannot claim the same variable in one caller.

Bindings do not create startup dependencies. Two apps may call each other without requiring each other to start first. A binding also does not grant access, forward a person's identity, or create a distributed transaction. The receiving app must enforce its own authentication, authorization, and retry contract.

## What the report establishes

The report lists app directories, packages, declared platforms, render mode, backend choice, managed image repositories, database declarations, bindings, and startup order. A `null` backend choice means that Cargo target discovery still needs to resolve it.

The JSON report has `format_version = 1` and a `checks` object. `configuration = "passed"` means the declarations passed the checks above. Artifact builds, API compatibility, physical database ownership, service identity, and deployment remain `"not_checked"`. A shared environment variable name such as `DATABASE_URL` does not prove two apps own different databases.

Inspection stops at this report. Processes released inside a single app remain part of that app's release.

## Release a set of independently built images

Build and publish each selected app's image through its verified packaging workflow first. Prepare the app's managed infrastructure and operator approval policy as described in the deployment and verification guides. Put the versions and independently authenticated descriptor hashes in a release set:

```toml
format_version = 1

[applications.accounts]
version = "accounts-v1"
descriptor = "apps/accounts/release.json"
descriptor-sha256 = "<reviewed descriptor SHA-256>"
setup = true

[applications.accounts.environment]
DATABASE_PASSWORD = "ACCOUNTS_DATABASE_PASSWORD"
POSTGRES_PASSWORD = "ACCOUNTS_POSTGRES_PASSWORD"

[applications.website]
version = "web-v1"
descriptor = "apps/website/release.json"
descriptor-sha256 = "<reviewed descriptor SHA-256>"
setup = true

[applications.website.environment]
DATABASE_PASSWORD = "WEB_DATABASE_PASSWORD"
POSTGRES_PASSWORD = "WEB_POSTGRES_PASSWORD"
```

Descriptor paths are relative to the workspace file. Environment entries map a deployment variable to the name of an operator environment variable; they never contain its value. Tool-control variables such as `PATH` cannot be mapped, even if declared as app secrets. Other required credentials still come from the operator environment. Use `setup = true` for an initial release and `maintenance = true` when the app's schema-changing release requires downtime.

```sh
lk workspace plan --releases releases.toml --out release-plan.json
lk workspace deploy --plan release-plan.json --sha256 <reviewed-plan-sha256>
```

Review the plan and retain its hash independently before deploying. Planning checks the workspace graph, application configurations, descriptor contents and migration identities. Each selected readiness dependency must also be in the release set, even if it will keep its current version. The deploy command then checks operator authorization and image/runtime evidence through the existing per-app release path. It uses direct managed deployment rather than a connected deployment service.

Apps run in readiness order. Each app keeps its own reservation, migration history, image, and release records. A failure stops the remaining commands and leaves already completed apps at their new versions; this is not a transaction across apps. Progress is saved beneath `.leptoskit/workspace`. Each app has a default 900-second local command deadline, configurable with `--timeout-seconds`; a timeout may leave a remote reservation that needs inspection through `lk infra status` before recovery.

Repeating the same plan revalidates completed apps. `deploy --reuse-current` checks the latest recorded version, descriptor, current approval policy, configuration, database history, running image, app environment, and readiness before leaving it untouched. A changed credential, unfinished operation, stopped container, or different current version refuses reuse. History created before configuration recording also requires a new version. The journal cannot skip these checks.

Plans freeze artifact and configuration choices. They do not yet build images, calculate source-change impact, create private network connections, inject the workspace's call bindings, or infer compatibility between separate apps. Configure caller-specific credentials and connectivity explicitly; the receiving application must still enforce identity and authorization. Runtime checks are operator observations, not runtime attestation.
