# leptoskit_cloud_app

<!-- {=cloudAppSummary} -->

Production binary (`leptoskit-cloud-control`) and hydrated web console for `LeptosKit` Cloud. Wraps `leptoskit_cloud`'s server routes, OAuth/OIDC device login, readiness probes, database migrations, operator bootstrapping, and S3/Stripe/SMTP environment validation.

<!-- {/cloudAppSummary} -->

Install with `cargo binstall leptoskit_cloud_app` (or `cargo install leptoskit_cloud_app --features production --locked`). Build production images with `--features production` (which enables `ssr` + `postgres` together), or run locally in development mode (once installed beside `lk` or on `PATH`, `lk cloud control <command>` dispatches to it automatically):

```sh
cargo run -p leptoskit_cloud_app --features server -- serve --dev
```

## Commands

| Command                   | What it does                                                                                                      |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `serve`                   | Serves the control plane. Refuses a database with pending migrations, or with migrations this build doesn't know. |
| `serve --dev`             | Development: see below. Development builds only.                                                                  |
| `migrate`                 | Applies pending migrations, SQLite or Postgres by the URL.                                                        |
| `operator grant <email>`  | Makes the person who verified `<email>` an operator. They must have signed in once. Recorded in the audit log.    |
| `operator revoke <email>` | Stops them being an operator. `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL` doesn't make them one again.             |
| `operator list`           | Lists operators by address.                                                                                       |

With `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL` set, `migrate` and `serve` make that address an operator once per database, while it has never had one: `operator revoke` stays final.

`serve` prints one line, `listening on <address>`, once it is listening, and never prints a secret. It stops on Ctrl-C or `SIGTERM`: `/readyz` fails first, the live socket drains, and requests in flight get 20 seconds.

## Environment

Every setting comes from the environment. Nothing secret has a default, and an error names the variable that is wrong without repeating its value.

| Variable                                | Required                | What it is                                                                                                                                                                                                                                                                                                       |
| --------------------------------------- | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `LEPTOSKIT_CLOUD_DATABASE_URL`          | always                  | `sqlite://path`, `sqlite::memory:`, or `postgres://…` (with the `postgres` feature).                                                                                                                                                                                                                             |
| `LEPTOSKIT_CLOUD_ORIGIN`                | `serve`                 | The public origin, such as `https://cloud.leptoskit.com`: `https`, no path. Sessions, passkeys, wallet messages, and mail links name it. In `--dev`, `http://<bind>`.                                                                                                                                            |
| `LEPTOSKIT_CLOUD_AUTH_KEYS`             | `serve`, except `--dev` | The keys sessions and codes are signed with: `id:key` pairs separated by commas, newest first, each key 32 random bytes in unpadded base64url.                                                                                                                                                                   |
| `LEPTOSKIT_CLOUD_VAULT_KEYS`            | `serve`, except `--dev` | The master keys that wrap each organisation's secrets key, in the same form, newest first. None of them may be a key in `LEPTOSKIT_CLOUD_AUTH_KEYS`, and a key stays listed while anything it wrapped is kept.                                                                                                   |
| `LEPTOSKIT_CLOUD_DNS_RESOLVER`          | optional                | The DNS-over-HTTPS resolver custom domains are verified through, which must answer `?name=…&type=TXT` in JSON. `https://cloudflare-dns.com/dns-query` without it; `https` except in `--dev`.                                                                                                                     |
| `LEPTOSKIT_CLOUD_SMTP_URL`              | `serve`, except `--dev` | Where mail goes: `smtps://user:password@host`, or `smtp://…:587?tls=required`.                                                                                                                                                                                                                                   |
| `LEPTOSKIT_CLOUD_MAIL_FROM`             | `serve`, except `--dev` | Who mail is from, such as `LeptosKit Cloud <cloud@leptoskit.com>`.                                                                                                                                                                                                                                               |
| `LEPTOSKIT_CLOUD_GITHUB_CLIENT_ID`      | with its secret         | GitHub sign-in's OAuth app. Both halves, or neither; without them GitHub sign-in is off.                                                                                                                                                                                                                         |
| `LEPTOSKIT_CLOUD_GITHUB_CLIENT_SECRET`  | with its ID             |                                                                                                                                                                                                                                                                                                                  |
| `LEPTOSKIT_CLOUD_GOOGLE_CLIENT_ID`      | with its secret         | Google sign-in's OAuth client, likewise.                                                                                                                                                                                                                                                                         |
| `LEPTOSKIT_CLOUD_GOOGLE_CLIENT_SECRET`  | with its ID             |                                                                                                                                                                                                                                                                                                                  |
| `LEPTOSKIT_CLOUD_BIND`                  | no; `127.0.0.1:8080`    | The address to listen on. Port 0 picks a free one.                                                                                                                                                                                                                                                               |
| `LEPTOSKIT_CLOUD_DEFAULT_REGION`        | no; `eu-central`        | Where projects run when they don't say.                                                                                                                                                                                                                                                                          |
| `LEPTOSKIT_CLOUD_APPS_DOMAIN`           | `serve`, except `--dev` | The domain environments are served under, such as `apps.leptoskit.dev`: production at `<project>-<organization>.<domain>`. In `--dev`, `apps.localhost`.                                                                                                                                                         |
| `LEPTOSKIT_CLOUD_REGISTRY`              | `serve`, except `--dev` | Where build workers push images, as a host and path with no scheme, such as `registry.example.com/leptoskit`. In `--dev`, `registry.localhost/leptoskit`.                                                                                                                                                        |
| `LEPTOSKIT_CLOUD_PRICE_CATALOGUE`       | no                      | A JSON price catalogue's path. Without it, `preview_release` says there is no price catalogue instead of estimating.                                                                                                                                                                                             |
| `LEPTOSKIT_CLOUD_CLIENT_IP_HEADER`      | no                      | The header the proxy in front of the control plane puts the client's address in, such as `x-real-ip`: one the proxy sets itself, to one address. `x-forwarded-for` and `forwarded` are refused. Rate limits count by it; without it they count by the connection's address, which behind a proxy is the proxy's. |
| `LEPTOSKIT_CLOUD_STRIPE_KEY`            | no                      | A restricted Stripe key, to sell plans: it writes customers, checkout sessions and customer portal sessions, and reads subscriptions and charges. With it, the next two and `LEPTOSKIT_CLOUD_PRICE_CATALOGUE` are required.                                                                                      |
| `LEPTOSKIT_CLOUD_STRIPE_WEBHOOK_SECRET` | with the key            | The secret Stripe signs the events it posts to `/webhooks/stripe` with.                                                                                                                                                                                                                                          |
| `LEPTOSKIT_CLOUD_STRIPE_PRICES`         | with the key            | Stripe's price for each plan that is sold, as `pro=price_…,team=price_…`.                                                                                                                                                                                                                                        |
| `LEPTOSKIT_CLOUD_STRIPE_AUTOMATIC_TAX`  | no                      | `true` to have Stripe work out tax at checkout. It collects none until the Stripe account is registered where tax is owed.                                                                                                                                                                                       |
| `LEPTOSKIT_CLOUD_STRIPE_TEST_MODE`      | no                      | `true` to take a Stripe test key outside `--dev`, for a staging control plane. With a test key nobody pays, and a made up card buys a plan, so a test key is refused without it.                                                                                                                                 |
| `LEPTOSKIT_CLOUD_S3_ENDPOINT`           | `serve`, except `--dev` | The S3-compatible store source archives and logs are kept in, such as `https://<account>.r2.cloudflarestorage.com`. Clients upload to it directly with presigned addresses.                                                                                                                                      |
| `LEPTOSKIT_CLOUD_S3_REGION`             | no; `auto`              | The store's region.                                                                                                                                                                                                                                                                                              |
| `LEPTOSKIT_CLOUD_S3_SOURCES_BUCKET`     | with the endpoint       | The bucket source archives go in.                                                                                                                                                                                                                                                                                |
| `LEPTOSKIT_CLOUD_S3_LOGS_BUCKET`        | with the endpoint       | The bucket build and runtime logs go in.                                                                                                                                                                                                                                                                         |
| `LEPTOSKIT_CLOUD_S3_ACCESS_KEY_ID`      | with the endpoint       | The store's credentials.                                                                                                                                                                                                                                                                                         |
| `LEPTOSKIT_CLOUD_S3_SECRET_ACCESS_KEY`  | with the endpoint       |                                                                                                                                                                                                                                                                                                                  |
| `LEPTOSKIT_CLOUD_S3_PATH_STYLE`         | no; `false`             | `true` addresses buckets as `endpoint/bucket`, as `MinIO` needs.                                                                                                                                                                                                                                                 |
| `LEPTOSKIT_CLOUD_STORAGE_DIR`           | `--dev` only            | Where `--dev` keeps objects instead of a store; by default a directory under the system's temporary directory.                                                                                                                                                                                                   |
| `LEPTOSKIT_CLOUD_SITE`                  | no                      | With `ssr`: the directory holding the console's client build in `pkg/`. Without it the console's pages render but don't hydrate.                                                                                                                                                                                 |

Sign-in offers email codes, passkeys, and Solana wallets always, and GitHub and Google when configured. Passwords are off.

## Development

`serve --dev` is for trying the control plane on your own machine, and a release build refuses it:

- it listens on, and is reached at, loopback only: `LEPTOSKIT_CLOUD_BIND` and `LEPTOSKIT_CLOUD_ORIGIN` must name this machine;
- `POST /auth/dev/sign-in` with `{"email": …}` signs anyone in without a code;
- mail stays in memory, listed in plain text at `GET /_leptoskit/mail`;
- without `LEPTOSKIT_CLOUD_AUTH_KEYS`, a random key is made for this run, so sessions end when it stops;
- without `LEPTOSKIT_CLOUD_VAULT_KEYS`, a random master key is made for this run, so secrets stored before it stops no longer open;
- without `LEPTOSKIT_CLOUD_S3_ENDPOINT`, objects are kept in `LEPTOSKIT_CLOUD_STORAGE_DIR` and uploaded and downloaded through the control plane itself, at addresses it signs and that expire;
- the database is migrated on start.

## Routes

| Path                               | What it is                                                                              |
| ---------------------------------- | --------------------------------------------------------------------------------------- |
| `POST /rpc/<operation>`            | The operations, with named JSON arguments.                                              |
| `/live`                            | The live socket.                                                                        |
| `/auth/…`                          | Sign-in.                                                                                |
| `/oauth/…`, `/.well-known/…`       | The OAuth provider, including the device flow the CLI signs in with.                    |
| `GET /.well-known/leptoskit-cloud` | The CLI's client ID, its scopes, and the operations' path.                              |
| `GET /openapi.json`                | The operations' OpenAPI document.                                                       |
| `GET /healthz`, `GET /readyz`      | Liveness, and readiness (which fails once shutdown starts).                             |
| `PUT`, `GET /_leptoskit/storage/…` | In `--dev` without a store, uploads and downloads at the addresses the operations sign. |
| `GET /sign-in`, `/welcome`, …      | With `ssr`: the console's pages, rendered and then hydrated.                            |
| `GET /_leptoskit/pkg/…`            | With `ssr` and `LEPTOSKIT_CLOUD_SITE`: the console's client.                            |
| `GET /_leptoskit/fonts/…`          | With `ssr`: the console's two faces.                                                    |

## The console

Built with `ssr`, the server also renders the console: leptoskit components in file routes under `src/routes/`, hydrated in the browser by the library built with `hydrate` for `wasm32-unknown-unknown`. There is no hand-written JavaScript; the only script on a page is leptos's loader for the Wasm. `docs/agents/cloud.md` says how to build the client and run both.
