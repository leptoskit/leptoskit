---
"leptoskit_notifications": patch
---

# Split the notifications guide and add Firebase migration

The notifications guide is now a chapter of twelve pages instead of one long page: an overview with a quick start, sending, the inbox, audiences, shaping messages, push setup, receiving on the device, delivery and troubleshooting, campaigns, privacy and data retention, coming from Firebase, and a reference.

- **Troubleshooting** lists every delivery reason code, every error the service answers with its HTTP status, the messages the service stops with at startup, and the enrollment and receiving errors in the app, each with what to do.
- **Coming from Firebase** maps Firebase Cloud Messaging's concepts and calls to this service, gives a migration in steps, and lists what is not there yet.
- **Reference** lists every operation with who may call it, every limit, every variable, and the settings in `leptoskit.toml`.
- The examples that no longer compiled are fixed: sends are built with `..SendSpec::default()`, a person's calls use `leptoskit_rpc::client::call`, and `RegisterPushEndpoint`, `PushDeliveries`, `Channel::new`, the service-worker route, and the receiver component match the current API.
- The signing and publishing pages now explain `[native.macos] provisioning-profile` and `LEPTOSKIT_MACOS_IDENTITY`, which a Mac app that receives push needs.

The crate's readme no longer says that nothing reports a tap to the service, and points at the chapter.
