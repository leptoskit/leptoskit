---
"leptoskit_notifications": minor
"leptoskit_plugin_notifications": minor
---

# Development and store builds share one service

Apple runs two push servers, and a device token is known to one of them. Until now the service used one for every device, so a build run from Xcode and a TestFlight build could not enroll with the same service. Now each iOS and macOS device says which server its build is signed for, and the service pushes it through that one.

- The iOS and macOS hosts answer a new plugin method, `APS_ENVIRONMENT` (method 11): the simulator answers development, a build with an embedded provisioning profile answers what the profile's `aps-environment` entitlement says, and an iOS build without a profile, which came through TestFlight or the App Store, answers production. Android does not answer it.
- `native::enroll` and the launch `reconcile` send the answer with the device's other details, as `DeviceDetails::apns_environment`. Your app writes no code for it, and a change reaches the service at the next launch.
- The service stores it per endpoint and picks Apple's server from it. `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` is now the default, for a device that reported nothing: an older app version, a Mac build without a profile, or an imported token.
- `import_push_endpoints` and a direct `register_push_endpoint` take the same field, so imported APNs tokens can name their environment.
- `person_endpoints` and `push_endpoints` show it under `device`. Conditions do not read it, and it is dropped for an FCM or Web Push endpoint.

The `.p8` key, the team id, and the topic are still one of each, so both builds need the same bundle id and a key that may use both servers.

This release has one migration per engine, `endpoint_apns_environment`, which adds a nullable column to `push_endpoints`. Run `leptoskit db --service notifications migrate` before you start the new version.

Breaking for code that builds these by hand: `DeviceDetails` has a new public field, so a struct literal needs `..DeviceDetails::default()`. `ApnsEnvironment` now lives in `push` and is re-exported from `service::config`, where it was. A host that predates method 11 answers "unavailable" for it, and the app enrolls as before.

The Swift code that reads the provisioning profile was not compiled or run on a device for this release.
