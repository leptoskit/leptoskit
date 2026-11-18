# Cloud CLI 1: `lk cloud login` and the account commands (specification)

The first part of the CLI for LeptosKit Cloud: `lk cloud login` and the account, organisation, project, environment and token commands. The control plane they talk to is `crates/leptoskit_cloud`, served by `crates/leptoskit_cloud_app`. Deploying (`launch`, `deploy`, `logs`, `status`) is a later slice: those commands are not added here.

## Read

- `cloud-architecture.md` (same directory as this file): §5 (`[cloud]` in `leptoskit.toml`), §6 (operations) and §9 (the first-deploy flow and where the CLI keeps credentials).
- `codebase-map.md` (same directory): section 2 (the RPC wire format: `POST {origin}{rpc_path}/{name}`, a JSON object of named arguments, the bare result, errors as `Kind|message` text with a status), section 3 (how the CLI already talks to a hosted API: `console/client.rs`, the credential file pattern, `ureq`, how subcommands are declared in `cli.rs` and dispatched), and section 4 (the identity provider's device flow endpoints).
- In the repository: `docs/agents/cloud.md` (identity order, the permission table with person-only permissions, error codes such as `sign_in_required`, `person_required`, `organization_not_found`, `insufficient_role`, `token_name_taken`, the discovery document at `GET /.well-known/leptoskit-cloud`, token formats and lifetimes); `crates/leptoskit_cloud/src/{account,projects,tokens,access}.rs`; `crates/leptoskit_cloud/tests/support/` (how the tests drive the device flow end to end); `crates/leptoskit_cli/src/console/client.rs` and `console/tests.rs`; `crates/leptoskit_cli/src/publish/secrets.rs` (the redacting `Secret` type) and `publish/tests.rs` (the fake-server test pattern); `crates/leptoskit_rpc_codegen/templates/cli_login.rs`; `crates/leptoskit_config/src/cloud.rs`.

## A. `crates/leptoskit_cli/src/cloud/`: a typed client and its commands

- **Which cloud.** In order: `--url`, `LEPTOSKIT_CLOUD_URL`, `[cloud].url` in the project's `leptoskit.toml` when there is one, then the default `https://cloud.leptoskit.dev`. Only HTTPS, or HTTP on loopback, with no path, query or credentials.
- **Client.** Blocking `ureq`, like the rest of the CLI. One function per operation that takes and returns the types from `leptoskit_cloud` (depended on with no features), so there is one source of truth for names and shapes. Error responses map to a typed error carrying the status and the stable code, and each known code renders as one plain sentence that says what to do next (for example `sign_in_required` → "You're not signed in. Run `lk cloud login`."). Unknown codes show the server's message. Timeouts on every request; no redirects followed; no proxy for loopback.
- **`lk cloud login`.** Reads the discovery document, starts the device authorisation with the first-party CLI client and its scopes, prints the verification URL and user code, opens the browser when one is available (`--no-browser` or a non-interactive terminal prints only), polls at the server's interval honouring `slow_down`, and stores the tokens. Signing in again replaces the stored session. `lk cloud logout` revokes the refresh token at the server and deletes the stored credentials, succeeding even when the server cannot be reached (it then says the local credentials were removed and the session may still be live).
- **Credentials at rest.** One private file per cloud origin under `$XDG_CONFIG_HOME/leptoskit/cloud/` (or `~/.config`), mode 0600 in a 0700 directory, written atomically, refused when its permissions are wider (with the command that fixes it). Tokens are held in the redacting `Secret` type and never printed, logged or put in an error. Access tokens are refreshed shortly before expiry and once after a 401; refresh tokens rotate and reuse ends the session at the server, so two `lk` processes must not refresh at the same time: a file lock around refresh-and-store, re-reading the file after acquiring it, with a test of the race. `LEPTOSKIT_CLOUD_TOKEN` (an API token, for CI) overrides the stored session for that invocation and is never written to disk.
- **Commands**, each with `--json` (stable, documented output) and a human format in the CLI's existing style (`ui.rs`):
  - `lk cloud whoami`
  - `lk cloud org list | create <name> [--slug]`, `lk cloud org members <org>`, `invite <org> <email> --role`, `invitations <org>`, `revoke-invitation`, `set-role`, `remove-member` (operations that are person-only on the server: with `LEPTOSKIT_CLOUD_TOKEN` set, say before calling that this needs `lk cloud login`)
  - `lk cloud project list [--org] | create <name> [--org --region] | show | delete <project> --confirm <slug>`
  - `lk cloud env list | create | update | delete` (the update sends the revision it read; a stale revision explains that someone else changed it and to retry)
  - `lk cloud token list | create <name> --scope … [--project] [--expires 30|90|365] | revoke` (the value is shown once, with a line saying so; a retry that hits `token_name_taken` prints the server's explanation, which names the existing token so it can be revoked)
  - `lk cloud link [--org] [--project]`: writes `[cloud] project` (and `organization` when it is not the personal one, and `url` when it is not the default) into `leptoskit.toml` with `toml_edit`, preserving formatting and comments, after confirming the project exists; refuses to overwrite a different existing link without `--force`.
- Organisation and project default from `[cloud]` in the current project when not given. Destructive commands never prompt in CI and require their confirmation flag everywhere.
- The command is registered in `cli.rs` beside `Console`/`Deploy`, with help text in the CLI's voice. `lk deploy` keeps its current behaviour.

## B. Tests

- A fake control plane (the `publish/tests.rs` pattern: axum on a loopback port on its own thread, recording requests) covering discovery, the device flow (pending, `slow_down`, denied, expired, approved), every command's request shape and rendering, every error code's message, refresh before expiry and after a 401, a rotated refresh token being stored, the refresh race between two processes (or two threads sharing the file through the lock), logout with and without a reachable server, the CI token override, and file permissions (created private; a too-open file refused).
- No secret (access token, refresh token, API token, device code) appears in any output, error, `Debug` string or snapshot, checked by a test that sets known values and searches everything the commands print. The one exception is `token create`, which prints the new token's value once.
- `insta` snapshots of the help output for `lk cloud` and its subcommands and of the human output of each command; the existing CLI help snapshots change where the top-level command list is shown: review and update them.
- One end-to-end test against the real control plane: a new test file in `crates/leptoskit_cloud_app/tests/` (only a new file there, plus the `Cargo.toml` dev-dependency on `leptoskit_cli`) that starts the real server in `--dev` on a loopback port, runs the CLI's login through its library functions with the device flow approved through the identity provider's endpoints the way the control plane's own tests do it, then `whoami`, `project create`, `token create`, and a call with that API token.
- `link` edits a fixture `leptoskit.toml` preserving comments and formatting, and the result parses and validates with `leptoskit_config`.

## C. Documentation

`docs/agents/cloud.md`: this part of CLI 1 marked built with its evidence, and the command tree, where credentials live, the refresh lock, the environment variables and the `--json` shapes documented. No public (`docs/src`) documentation and no skill reference (the cloud is not available to app authors yet).

## Acceptance

- `RUSTUP_AUTO_INSTALL=0 CARGO_BUILD_JOBS=4 devenv shell -- cargo nextest run -p leptoskit_cli cloud` passes, as do the CLI's snapshot tests (`cargo nextest run -p leptoskit_cli --test snapshots`) and the end-to-end test (`cargo nextest run -p leptoskit_cloud_app --all-features <the test>`).
- `cargo clippy -p leptoskit_cli --all-features --all-targets -- -D warnings` passes.
- `devenv shell lint:format`, `lint:monostyle`, `lint:releases`, `lint:docs`, `deny:check` pass.
- A changeset for `leptoskit_cli` (minor), mirroring an earlier CLI changeset's frontmatter.

## Leave alone

`crates/leptoskit_cloud` beyond what the task you were given explicitly permits, `crates/leptoskit_cloud_app` other than the one new test file and its `Cargo.toml` dev-dependency, `leptoskit_config`, `leptoskit_auth`, `leptoskit_idp`, `leptoskit_rpc`, the `publish` and `console` modules' behaviour, and `lk deploy`. Not added here: `launch`, `deploy`, `status`, `logs`, `open`, `secret`, `domain`, `scale`, `rollback`, `db`, `publish`, `usage`.
