---
"leptoskit_notifications": minor
---

# Push transports follow each provider's documented rules

The APNs, FCM, and Web Push transports were checked against Apple's, Google's, and Microsoft's documentation and the Web Push RFCs, and corrected where they disagreed:

- **Safari and Edge can enroll.** Web Push now accepts any subdomain of `push.apple.com` (Safari's `web.push.apple.com`, as Apple asks senders to allow) and of `notify.windows.com` (the Windows Push Notification Services channels Edge on Windows hands out), matched on whole labels. The never-used `webpush.apple.com` is no longer accepted, and lookalikes, other ports, and credentials in the URL are refused as before.
- **FCM measures what FCM counts.** The 4,096-byte limit is checked on the keys and values a device receives, not the whole HTTP request, and the APNs block no longer repeats the title and body unless a subtitle needs them. The longest title and body a send allows now reach an Android device instead of ending `oversized`. The degradation order is the same on all three transports: the per-platform blocks and the image, then sound, badge, and link, then the send's `data`.
- **Silent means silent.** A data message carries no notification block on any provider: no `android.notification` at FCM, which would show it in the tray, and nothing but `content-available` in APNs's `aps`. A data send's badge rides as `lkbadge` data for the app to apply. `image`, `sound`, `link`, and the `android`, `apple`, and `web` blocks are now refused on a data send, naming the field.
- **Data messages are never pushed to a browser.** Browsers require every push to show a notification, so a data send's Web Push deliveries end `skipped` with the new reason `platform_unsupported`, and its native deliveries go as usual.
- **Only a gone device is invalidated.** APNs `410`, FCM `UNREGISTERED`, and a push service's `404`/`410` invalidate an endpoint. APNs `BadDeviceToken` and `DeviceTokenNotForTopic` (a wrong environment or topic), FCM `SENDER_ID_MISMATCH` and its third-party credential errors, and a refused VAPID signature end the delivery as `operator_configuration` and leave the device enrolled. A provider token APNs refuses is dropped from the cache and the push retried under a fresh one.
- **FCM's reserved data keys** now include `message_type` beside `from`, `google.*`, and `gcm.*`; `lkbadge` joins the envelope's own keys. They are dropped from the FCM copy only.
- **A Web Push subscription whose key is no point on the curve** ends `skipped` with the new reason `subscription_unusable` instead of `oversized`.
- **Configuration.** Setting `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` without a VAPID pair now refuses to start, naming the missing variables. The guide shows the `secrets` list the `[services.notifications]` table needs for the push variables to reach the service, names every variable in full, and shows how to set a `.p8` key and a service-account JSON and how to generate a VAPID pair.
