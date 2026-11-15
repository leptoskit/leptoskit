---
"leptoskit_cli": minor
---

Opt-in APNs for generated iOS apps. `[native.ios] aps-environment = "development"` (or `"production"`) writes the `aps-environment` entitlement — Apple's exact string, not a boolean — into `.leptoskit/ios/App.entitlements`, and the generated app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` to the notifications plugin's broker. It must match the app's provisioning profile and the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. Apps without the setting generate exactly as before, with no push entitlement, so local-only notifications users stay local-only; no provisioning or certificate access happens in the build. The configuration schema carries the new key.
