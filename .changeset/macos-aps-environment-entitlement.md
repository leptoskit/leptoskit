---
"leptoskit_cli": minor
---

Opt-in APNs for generated macOS apps. `[native.macos] aps-environment = "development"` (or `"production"`) writes the `com.apple.developer.aps-environment` entitlement — Apple's macOS key, its exact string — into `.leptoskit/macos/App.entitlements`; the local build's ad-hoc signature and a `leptoskit publish macos` distribution both sign the app with it, and the generated macOS app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` to the notifications plugin's broker. It must match the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. Apps without the setting generate exactly as before, with no push entitlement, so local-only notifications users stay local-only.
