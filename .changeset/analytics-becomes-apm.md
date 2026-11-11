---
"release": minor
---

# Route metrics are now `leptoskit_apm`

The in-process route metrics that `leptoskit_analytics` held (minute aggregates, request correlation, the browser module at `/_leptoskit/analytics.mjs`, and OTLP export) are now the `leptoskit_apm` crate, and `leptoskit`'s `apm` feature. The name `leptoskit_analytics` is moving to the analytics service, a product-analytics service plugin.

Update your `leptoskit` dependency's features from `analytics` to `apm`, and `leptoskit::analytics::…` to `leptoskit::apm::…`. Both old names still work for this release. The endpoints, `[infra.plugins.analytics]`, and `lk infra analytics` are unchanged.
