---
"leptoskit_notifications": minor
---

Android FCM device-token enrollment, in the same `native` module as iOS. The platform is a transport choice and nothing more: a snapshot whose latest answer is an FCM registration token registers as the typed `PushCredential::Fcm` credential, an APNs hex token registers as `Apns`, and the enroll/reconcile/disable workflow around them — gesture enrollment, nonprompting launch reconciliation with rotation and owner-verified retirement, stable disable keys — is one shared implementation, not a fork. FCM tokens validate as 1–4096 bytes of printable ASCII, never hex, and both token kinds keep redacted `Debug` output. The plugin's snapshot type is renamed `RemoteRegistration` (from `ApnsRegistration`) now that it carries either platform's answer, with a new `RemoteState::FcmToken` variant on wire state 4.
