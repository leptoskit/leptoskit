# Applications in one workspace

Keep a website and an accounts API in the same repository while giving each its own application configuration. A `leptoskit.workspace.toml` file describes those boundaries and checks how the apps depend on one another before a build starts.

This command runs on the development machine. It reads configuration for web, server, and native apps; it does not build a platform host, connect to a database, or deploy anything.

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

The command finds the nearest workspace file in the current directory or its parents, so it also works from inside either app. Use `--manifest path/to/leptoskit.workspace.toml` to name a file explicitly. `check` exits with an error for invalid app configuration, unknown dependencies, startup cycles, repeated app configuration paths, or conflicting managed infrastructure names, domains, and image repositories.

## Readiness and calls are different

`depends-on` means that a provider must be ready before its consumer starts. These edges must have no cycles. The report gives a deterministic startup order.

A binding describes a call from one app to another. `url-env` and `token-env` are variable names for that caller. The command never reads their values. Binding names and destination apps are checked, and two binding fields cannot claim the same variable in one caller.

Bindings do not create startup dependencies. Two apps may call each other without requiring each other to start first. A binding also does not grant access, forward a person's identity, or create a distributed transaction. The receiving app must enforce its own authentication, authorization, and retry contract.

## What the report establishes

The report lists app directories, packages, declared platforms, render mode, backend choice, managed image repositories, database declarations, bindings, and startup order. A `null` backend choice means that Cargo target discovery still needs to resolve it.

The JSON report has `format_version = 1` and a `checks` object. `configuration = "passed"` means the declarations passed the checks above. Artifact builds, API compatibility, physical database ownership, service identity, and deployment remain `"not_checked"`. A shared environment variable name such as `DATABASE_URL` does not prove two apps own different databases.

Workspace inspection currently stops at this report. It does not inject bindings into running apps, choose which apps to rebuild, create separate images, or orchestrate releases. Use each app's existing build and release commands until those operations consume the workspace graph. Processes released inside a single app remain part of that app's release.
