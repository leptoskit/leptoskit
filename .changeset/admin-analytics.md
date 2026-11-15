---
"leptoskit_admin": minor
---

# Analytics in the admin

The admin's Analytics area now reads your site's analytics. Connect it with `.analytics(AnalyticsService::new(url, token))` on the builder (or `AnalyticsService::from_env()`, which reads `LEPTOSKIT_SERVICE_ANALYTICS_URL` and `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`). The admin calls the analytics service as the app, signed with the service's token, which stays on the admin's server. Without a service, every Analytics page shows how to connect one.

The pages:

- **Overview**: visitors, pageviews, sessions, and bounce rate against the previous period, plus top and entry pages, referrers, channels, UTM, devices, browsers, and operating systems.
- **Trends**: a builder over the events your app has sent, with a `struct` badge on typed ones, and controls for math, breakdown, interval, compare, and range.
- **Funnels**: conversion and the median time to convert.
- **Retention**: a heat grid of cohorts.
- **People**: search and a property filter. Each person's page shows their timeline, ids, and sessions, with export and delete behind an in-page confirmation.
- **Live**: an event tail with a filter and pause. The top bar's pulse follows it.
- **Library**: saved insights and dashboards that update as events arrive.

Every point, step, drop-off, and retention cell opens onto the people behind it. Every insight's **Workings** drawer follows its number through five stations:

- **Emit**: the Rust type.
- **Send**: the bytes it sends.
- **Store**: the tables it lands in.
- **Ask**: the query as Rust, SQL, and HTTP, updating as you change the builder.
- **Draw**: the rows the chart was drawn from.

Learn's analytics lessons now tick from your real data.
