---
"release": minor
---

# Record analytics events in the browser

With the `web` feature, `leptoskit_analytics::client::web` records events from a web app's pages to the analytics service on the app's own origin:

```rust
use leptoskit_analytics::client::web;
use leptoskit_analytics::props;

web::init(web::Options::default());
web::capture_pageview(props! {});
web::capture("signed_up", props! { "plan" => "pro" });
web::identify(&user_id, props! { "email" => email });
```

It remembers the person and whether they opted out in `localStorage`, sends every few seconds with `fetch`, and hands what's left to `sendBeacon` when the page is hidden or closed, so the last events aren't lost. It adds only `wasm-bindgen`, `js-sys`, and `web-sys`, which a leptoskit web app already compiles.

The client's queue now also counts a batch on its way and the batches' shared details against its limits, and drops an event too large to ever send when it's recorded rather than holding it.
