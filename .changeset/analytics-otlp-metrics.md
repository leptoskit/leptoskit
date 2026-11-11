---
"release": minor
---

# Store route timings in the analytics service

The analytics service accepts application-performance metrics at `/_leptoskit/services/analytics/otlp/v1/metrics`, in the OTLP/HTTP JSON that `leptoskit_apm`'s exporter sends, so how long each route took is kept beside the product analytics. Each minute of each route is stored once, even when the exporter retries. Only the app may send them, signed with the service's token. Metrics the service doesn't store, such as cumulative histograms or gauges, are reported back as rejected, as OTLP expects.
