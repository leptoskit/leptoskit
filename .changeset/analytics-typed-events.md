---
"release": minor
---

# Declare analytics events as Rust types

With `leptoskit_analytics`'s `derive` feature, an app can declare an event as a struct, so a misspelled or retyped property is a compile error instead of a gap in the data:

```rust
#[derive(leptoskit_analytics::Event)]
#[event(name = "checkout_completed")]
struct CheckoutCompleted {
	plan: String,
	amount_cents: u32,
}

client.capture_event(CheckoutCompleted { plan: "pro".into(), amount_cents: 1999 });
```

Each field becomes a property of the same name. Without `name`, the event is named after the struct. The app's server records them with `Analytics::capture_event`. The derive refuses names the service would refuse (blank, longer than 200 bytes, or with control characters such as newlines) and names starting with `$`, which leptoskit's own events use, as well as tuple structs and enums, saying what to write instead. A field written `r#type` is the property `type`.

After initializing the browser or native client, record the same event with `client::web::capture_event` or `client::native::capture_event`. Both use the shared typed-event implementation.
