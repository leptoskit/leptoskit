# Run an admin for your site

Once your app is deployed, someone has to look after it: apply infrastructure changes, ship and roll back releases, check that backups restore, and see how the app is doing. The admin is one web app for all of that, and it signs people in through your site's own sign-in, so there's no second password to manage and no shared owner key on anyone's laptop.

The admin is a separate app with its own address, such as `admin.notes.example`. It runs in a web browser on any device, including phones. Your app's own clients on web, iOS, Android, macOS, Windows, Linux, the terminal, HarmonyOS (experimental), and Wear OS / watchOS (experimental) don't change.

## What's inside

- **Infrastructure** shows what's serving, the changes waiting in `leptoskit.toml`, releases, operations, health, backups, and the configuration. Plan, apply, deploy, first release, maintenance, roll back, health checks, and backup verification each ask you to confirm, run on the [deployment service](28-deployment-service.md)'s worker, and are recorded under your name. The page follows each one live, and the work carries on if you close the tab.
- **Analytics** teaches until your app sends its first typed event. It never draws charts from nothing.
- **Learn** has short lessons that tick themselves when they're true of your projects: you applied a plan, a backup restored, a health check passed.

Every card has a **Workings** drawer that walks what's on screen back to where it comes from: the `leptoskit.toml` lines that declare it, the `main.tf.json` they generate, the plan OpenTofu makes, the command the worker runs, and the check that says it's done. Press `W` to open it, and `Shift+W` to keep each card's source showing.

## Who gets in

Your site decides. It gives some accounts the `admin` role with a `Roles` hook, and sends roles to the admin when someone signs in:

```rust
impl auth::Roles for User {
    async fn roles(db: &Db, account: Id<Account>) -> auth::Result<Vec<String>> {
        match User::find(db, account).await? {
            Some(user) if user.admin => Ok(vec!["admin".into()]),
            _ => Ok(Vec::new()),
        }
    }
}
```

Someone without the role who presses **Continue** is told who they signed in as, and can switch account; your site asks them to sign in again. The admin reads roles at sign-in, so a role you give or take away applies from the next one.

## Register the admin with your site

Your site needs [sign-in](23-sign-in.md) with the `Roles` hook above, and the [identity provider](25-sign-in-with-your-app.md) so other apps can sign in with it. Register the admin as one of your own apps, so it skips the consent screen and may ask for roles:

```sh
lk auth idp client add "Notes admin" --first-party --secret-post \
  --redirect https://admin.notes.example/auth/callback/main \
  --post-logout-redirect https://admin.notes.example/_leptoskit/admin/sign-in \
  --scopes openid,email,profile,roles
```

The command prints the client ID and, once, its secret. Keep the secret in your secret store.

## Start the admin

Run it on the same trusted machine as your [deployment service](28-deployment-service.md), with its three secrets in the environment:

- `LEPTOSKIT_ADMIN_CLIENT_SECRET`: the secret your site just issued.
- `LEPTOSKIT_ADMIN_KEYS`: the admin's own session keys, in the same `id:key` form as `AUTH_ENCRYPTION_KEYS`.
- `LEPTOSKIT_CONTROL_KEY`: the deployment service's owner credential.

```sh
lk admin serve --project /srv/notes \
  --url https://admin.notes.example \
  --main-site https://notes.example \
  --client-id <client id>
```

Repeat `--project` for each repository. The admin listens on `127.0.0.1:4801`; put your TLS proxy or Cloudflare Tunnel in front of it, at the address `--url` names. It runs the deployment service and its worker inside itself, so stop `lk console serve` for those repositories first. Its database and the deployment journal live in `~/.local/share/leptoskit/console` unless you pass `--data`.

`--role ops` lets in a different role (repeat it to accept several), `--environment staging` labels a staging admin, and `--app-name Notes` names it in the top bar.

## Embed it in your own server

The admin is also a library, `leptoskit_admin`, for when you'd rather run it from your own binary:

```rust
use leptoskit_admin::Admin;
use leptoskit_admin::MainSite;

let admin = Admin::builder("Notes")
    .origin("https://admin.notes.example")
    .main_site(MainSite::new("https://notes.example", client_id, client_secret))
    .keys(leptoskit_auth::Keys::from_env("LEPTOSKIT_ADMIN_KEYS")?)
    .data_dir("/var/lib/notes-admin")
    .control(service)
    .build()
    .await?;
axum::serve(listener, admin.router()).await?;
```

`service` is the deployment service's `control::Service`. Start its worker as `lk console serve` does, or operations wait in the queue.

## Try it on your machine

The `admin_demo` example runs a site and its admin side by side, with two people: Ada, who is an admin, and Grace, who isn't. From a checkout of leptoskit:

```sh
cargo run -p admin_demo
```

Open `http://127.0.0.1:4801/_leptoskit/admin` and continue as either of them. The demo's operations are simulated and contact no cloud provider: a plan runs the real generator and compares the result with what it last applied, and every operation says it's a simulation. `cargo run -p admin_demo -- --executor lk` runs the real `lk` commands instead; without provider tokens, plans fail, and the admin shows the failure as it is.

## What it doesn't do yet

- Analytics screens teach and wait; charts for trends, funnels, retention, and people arrive with product analytics.
- The budget card shows the limit `[infra.budget]` sets, without a cost estimate.
- The admin manages the repositories passed to it with `--project`; adding one means restarting it.
