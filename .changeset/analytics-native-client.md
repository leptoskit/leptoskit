---
"release": minor
---

# Record analytics events in native apps, offline too

With the `native` feature, `leptoskit_analytics::client::native` records events from a native app's interface and sends them to the app's server through the HTTP plugin:

```rust
use leptoskit_analytics::client::native;
use leptoskit_analytics::props;

native::init(native::Options::new("https://app.example")).await;
native::capture_screen("Settings", props! {});
native::capture("exported", props! { "format" => "pdf" });
```

It sends every 30 seconds or 20 events. What it hasn't sent is kept in the app's private files, so events recorded offline, or just before the app closed, are sent the next time it runs, recorded at the time they happened and stored once. Call `native::flush().await` when the app goes to the background. The app needs the `http`, `filesystem`, and, for unpredictable ids, `crypto` plugins.
