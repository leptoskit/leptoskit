# Run an admin for your site

Once your app is deployed, someone has to look after it: apply infrastructure changes, ship and roll back releases, check that backups restore, and see how the app is doing. The admin is one web app for all of that, and it signs people in through your site's own sign-in, so there's no second password to manage and no shared owner key on anyone's laptop.

The admin is a separate app with its own address, such as `admin.notes.example`. It runs in a web browser on any device, including phones. Your app's own clients on web, iOS, Android, macOS, Windows, Linux, the terminal, HarmonyOS (experimental), and Wear OS / watchOS (experimental) don't change.

## What's inside

- **Infrastructure** shows what's serving, the changes waiting in `leptoskit.toml`, releases, operations, health, backups, and the configuration. Plan, apply, deploy, first release, maintenance, roll back, health checks, and backup verification each ask you to confirm, run on the [deployment service](28-deployment-service.md)'s worker, and are recorded under your name. The page follows each one live, and the work carries on if you close the tab.
- **Analytics** reads your site's [product analytics](40-product-analytics.md): the web overview, trends, funnels, retention, people, a live event tail, and saved insights on dashboards. Before it's connected, it shows how to connect it rather than empty charts.
- **Learn** has short lessons that tick themselves when they're true of your data: you applied a plan, a backup restored, a typed event arrived, you saved a funnel.

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

## Connect analytics

The admin is its own app, so it reads analytics the way your site's server does: it calls your site's analytics service as the app, signed with the service's token. Give it the service's address, one the admin's machine can reach, and the same token your site's server uses:

```sh
export LEPTOSKIT_SERVICE_ANALYTICS_TOKEN=…   # the same token as the site's server
lk admin serve --project /srv/notes \
  --url https://admin.notes.example \
  --main-site https://notes.example \
  --client-id <client id> \
  --analytics-url http://10.0.0.2:4800
```

`--analytics-url` defaults to `LEPTOSKIT_SERVICE_ANALYTICS_URL`. The token only ever lives on the admin's server; pages never see it, and with it the admin can read every event, so keep it in your secret store. Without both, every Analytics page explains how to connect.

Once connected, these pages read your real data:

- **Overview**: visitors, pageviews, sessions, and bounce rate against the previous period, with top and entry pages, referrers, channels, UTM sources, devices, browsers, and operating systems. Every site gets this without setup.
- **Trends**: pick events from the ones your app has sent (events with a Rust type wear a `struct` badge), count them in total, by person, or by session, split them by a property, and compare with the previous period. Press a point to see the people behind it.
- **Funnels**: the steps people take in order, how many reach each, the median time between them, and who dropped off.
- **Retention**: a grid of people who started in each day or week and how many came back. Press a cell to see who.
- **People**: search by email or name, filter by a property, and open someone to see their properties, ids, timeline, and sessions. Export and delete are there too, and each asks you to type the person's name first.
- **Live**: events a few seconds after they happen, with a filter and a pause. The pulse in the top bar follows it on every page.
- **Library**: save any insight, add it to a dashboard, and watch dashboards update as events arrive.

Every chart's **Workings** drawer follows a number from end to end: the Rust type your app emits, the bytes it sends, the tables it lands in, the query as Rust, SQL, and HTTP (which change as you change the builder), and the rows that became the chart.

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

`service` is the deployment service's `control::Service`. Start its worker as `lk console serve` does, or operations wait in the queue. Add `.analytics(AnalyticsService::new(url, token))` (or `AnalyticsService::from_env()`, which reads the two variables above) to connect analytics.

The admin's pages are moving from a hand-written script to Rust compiled to Wasm, a page at a time. The pages that have moved need their browser half, built once from a checkout of leptoskit:

```sh
cd crates/leptoskit_admin_pages
leptoskit build --release --platform web --out /var/lib/notes-admin/client
```

Pass that directory to `lk admin serve --client-dir` (or set `LEPTOSKIT_ADMIN_CLIENT_DIR`), or to the library with `.client_dir("/var/lib/notes-admin/client")`. Without it, those pages render and stay as the server drew them: links work, but the theme switch, the account menu and search don't. So far that is Learn, its lessons, the Glossary, Projects, each operation's page, every project tab except Health, the analytics Library, People, and each person's page.

## Try it on your machine

The `admin_demo` example runs a site and its admin side by side, with two people: Ada, who is an admin, and Grace, who isn't. From a checkout of leptoskit:

```sh
cargo run -p admin_demo             # infrastructure, with analytics waiting for events
cargo run -p admin_demo -- --seed   # and a month of analytics
```

Open `http://127.0.0.1:4801/_leptoskit/admin` and continue as either of them. The demo runs the real analytics service inside the main site, at `http://localhost:4800`. Its pages send typed events as you click around, so they show up under **Live**, and `--seed` first sends 30 days of visits, sign-ups, notes, and checkouts through the same ingestion your app's events use. The data lives in your temporary folder and starts fresh each run. The demo's operations are simulated and contact no cloud provider: a plan runs the real generator and compares the result with what it last applied, and every operation says it's a simulation. `cargo run -p admin_demo -- --executor lk` runs the real `lk` commands instead; without provider tokens, plans fail, and the admin shows the failure as it is.

## What it doesn't do yet

- Funnels show the median time for the whole funnel, not for each step.
- Analytics has no cohorts, actions, or SQL editor yet; the Workings drawer shows each query's SQL, but you can't edit it.
- The budget card shows the limit `[infra.budget]` sets, without a cost estimate.
- The admin manages the repositories passed to it with `--project`; adding one means restarting it.
