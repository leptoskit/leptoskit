# leptoskit_cloud_app

LeptosKit Cloud's control plane, as a program: `leptoskit-cloud-control`. The control plane itself (its operations, sign-in, the CLI's device login, and its routes) is `leptoskit_cloud`'s `server` feature; this crate reads the operator's environment, refuses a configuration that is incomplete or unsafe, and runs the commands. It is not published.

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
| `operator revoke <email>` | Stops them being an operator.                                                                                     |
| `operator list`           | Lists operators by address.                                                                                       |

`serve` prints one line, `listening on <address>`, once it is listening, and never prints a secret. It stops on Ctrl-C or `SIGTERM`: `/readyz` fails first, the live socket drains, and requests in flight get 20 seconds.

## Environment

Every setting comes from the environment. Nothing secret has a default, and an error names the variable that is wrong without repeating its value.

| Variable                               | Required                | What it is                                                                                                                                                            |
| -------------------------------------- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `LEPTOSKIT_CLOUD_DATABASE_URL`         | always                  | `sqlite://path`, `sqlite::memory:`, or `postgres://…` (with the `postgres` feature).                                                                                  |
| `LEPTOSKIT_CLOUD_ORIGIN`               | `serve`                 | The public origin, such as `https://cloud.leptoskit.com`: `https`, no path. Sessions, passkeys, wallet messages, and mail links name it. In `--dev`, `http://<bind>`. |
| `LEPTOSKIT_CLOUD_AUTH_KEYS`            | `serve`, except `--dev` | The keys sessions and codes are signed with: `id:key` pairs separated by commas, newest first, each key 32 random bytes in unpadded base64url.                        |
| `LEPTOSKIT_CLOUD_SMTP_URL`             | `serve`, except `--dev` | Where mail goes: `smtps://user:password@host`, or `smtp://…:587?tls=required`.                                                                                        |
| `LEPTOSKIT_CLOUD_MAIL_FROM`            | `serve`, except `--dev` | Who mail is from, such as `LeptosKit Cloud <cloud@leptoskit.com>`.                                                                                                    |
| `LEPTOSKIT_CLOUD_GITHUB_CLIENT_ID`     | with its secret         | GitHub sign-in's OAuth app. Both halves, or neither; without them GitHub sign-in is off.                                                                              |
| `LEPTOSKIT_CLOUD_GITHUB_CLIENT_SECRET` | with its ID             |                                                                                                                                                                       |
| `LEPTOSKIT_CLOUD_GOOGLE_CLIENT_ID`     | with its secret         | Google sign-in's OAuth client, likewise.                                                                                                                              |
| `LEPTOSKIT_CLOUD_GOOGLE_CLIENT_SECRET` | with its ID             |                                                                                                                                                                       |
| `LEPTOSKIT_CLOUD_BIND`                 | no; `127.0.0.1:8080`    | The address to listen on. Port 0 picks a free one.                                                                                                                    |
| `LEPTOSKIT_CLOUD_DEFAULT_REGION`       | no; `eu-central`        | Where projects run when they don't say.                                                                                                                               |

Sign-in offers email codes, passkeys, and Solana wallets always, and GitHub and Google when configured. Passwords are off.

## Development

`serve --dev` is for trying the control plane on your own machine, and a release build refuses it:

- it listens on, and is reached at, loopback only: `LEPTOSKIT_CLOUD_BIND` and `LEPTOSKIT_CLOUD_ORIGIN` must name this machine;
- `POST /auth/dev/sign-in` with `{"email": …}` signs anyone in without a code;
- mail stays in memory, listed in plain text at `GET /_leptoskit/mail`;
- without `LEPTOSKIT_CLOUD_AUTH_KEYS`, a random key is made for this run, so sessions end when it stops;
- the database is migrated on start.

## Routes

| Path                               | What it is                                                           |
| ---------------------------------- | -------------------------------------------------------------------- |
| `POST /rpc/<operation>`            | The operations, with named JSON arguments.                           |
| `/rpc/live`                        | The live socket.                                                     |
| `/auth/…`                          | Sign-in.                                                             |
| `/oauth/…`, `/.well-known/…`       | The OAuth provider, including the device flow the CLI signs in with. |
| `GET /.well-known/leptoskit-cloud` | The CLI's client ID, its scopes, and the operations' path.           |
| `GET /openapi.json`                | The operations' OpenAPI document.                                    |
| `GET /healthz`, `GET /readyz`      | Liveness, and readiness (which fails once shutdown starts).          |
