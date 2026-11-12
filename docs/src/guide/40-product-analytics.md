# Product analytics

You want to know what people do in your app: who signed up, which screens they use, where a checkout gets abandoned. Sending that to a third-party analytics product means shipping your users' activity to someone else's servers, and running the self-hosted ones means a cluster of databases and queues. leptoskit has an analytics **service** you add with one table in `leptoskit.toml`: a small program with its own database that runs beside your app's server and records events.

This page shows how to add it and record events from your app's server. The service runs on the server side, wherever your app's server does. Recording from the browser and from native apps (iOS, Android, macOS, Windows, Linux, and the terminal) comes in a later release; until then, record those events through your server, or post them to the service as JSON (below).

## Add the service

Name it in your app's `leptoskit.toml`:

```toml
[services.analytics]
crate = "leptoskit_analytics"
version = "0.3"
```

That's all `leptoskit dev` and `leptoskit serve` need: they build the service, give it a database of its own on the same engine as your app's (SQLite, or Postgres when your app uses Postgres), start it beside your app, and answer for it at `/_leptoskit/services/analytics/` on your app's own address. [Services](32-services.md) explains how services run and deploy.

If your app's database is Postgres, build the service with Postgres support:

```toml
[services.analytics]
crate = "leptoskit_analytics"
version = "0.3"
features = ["service", "postgres"]
```

## Record events from your server

Add the client to your server's dependencies:

```toml
[dependencies]
leptoskit_analytics = { version = "0.3", features = ["server-client"] }
```

Then record events where they happen, naming whose they are:

```rust
use leptoskit_analytics::client::server::Analytics;
use leptoskit_analytics::props;

// Once, when the server starts. `leptoskit dev` and `leptoskit serve` tell it
// where the service is; `None` means this app doesn't run the service.
let analytics = Analytics::from_env().expect("the app runs the analytics service");

analytics.capture(&user_id, "invoice_paid", props! {
    "amount_cents" => 1999,
    "currency" => "EUR",
});
analytics.identify(&user_id, &anonymous_id, props! { "email" => email });
analytics.group_identify("company", &company_id, props! { "name" => company_name });
```

Recording never waits: each call queues the event and returns, and the client sends what's queued every second, or as soon as a batch's worth is waiting, in a compact binary format. Before your server exits, `analytics.flush().await` waits until what's queued is stored.

The client signs what it sends with the service's token, which only your server has. The service records these events as your server's, and never limits how fast your server sends them.

## Record events from anywhere else

Anything that can send JSON can record events, which is useful from a script, a job in another language, or `curl`:

```sh
curl -X POST https://app.example/_leptoskit/services/analytics/api/capture \
  -H 'content-type: application/json' \
  -d '{"batch":[{"event":"signed_up","distinct_id":"user-1","properties":{"plan":"pro"}}]}'
```

The answer says how many events were stored, how many were already stored by an earlier attempt, and which were refused and why:

```json
{ "stored": 1, "duplicates": 0, "rate_limited": 0, "refused": [] }
```

## What the service promises

- **An answer means it's stored.** The service answers once a batch's events are written, so a client that got an answer can forget them. Many requests share each write, which keeps it fast.
- **Retries are stored once.** Every event has an id made when it happened, so a batch sent twice, after a timeout say, is stored once and counted as duplicates.
- **Nothing is dropped silently.** An event that can never be stored (no name, properties too large, an id that isn't valid) is refused with a reason in the answer, and counted in the service's records.
- **A busy service says so.** When its queue is full, the service answers `503` with `Retry-After` rather than dropping events, and the clients wait and retry.
- **One sender can't crowd out the rest.** Each person's id may send 100 events a second, in bursts of up to 2,000, far more than a real device sends; past that, events are dropped and counted as `rate_limited`.
- **Clocks are corrected.** An event's time is adjusted for how far the sending device's clock is off, and a time still more than 23 hours ahead after that is replaced by the time the service received it.

If your app prefers the fastest answer to the strongest promise, set `LEPTOSKIT_ANALYTICS_ACK=queued` in the service's environment: it then answers as soon as the events are queued, and a crash loses at most what was queued.

## Route timings

The [application performance](27-analytics.md) package measures how long each of your routes takes. The analytics service stores those measurements too, so they sit beside your product analytics. Point the package's export at the service, in your app server's environment:

```sh
LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT=<the service's private URL>/_leptoskit/services/analytics/otlp/v1/metrics
LEPTOSKIT_ANALYTICS_EXPORT_PROTOCOL=otlp
LEPTOSKIT_ANALYTICS_WRITE_KEY=<the service's token>
```

`leptoskit dev` and `leptoskit serve` give your server the service's private URL and token as `LEPTOSKIT_SERVICE_ANALYTICS_URL` and `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. Each minute of each route is stored once, even when the export is retried, and only your server may send them.
