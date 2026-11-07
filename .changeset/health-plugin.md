---
"release": minor
---

# Add the health plugin: Health Connect and HealthKit reads

Add a `leptoskit_plugin_health` crate with a typed binary guest client, HealthKit and Health Connect handlers, and automatic native registration under the runtime name `health`.

Read heart rate, resting heart rate, heart-rate variability (RMSSD on Android, SDNN on iOS, never converted), steps, sleep with stages, oxygen saturation, respiratory rate and exercise sessions, each with its source app and device. Permission query and request go through the platform's own flows, reads page through a token, and incremental sync stores an opaque change token: Health Connect's `getChanges` tokens, HealthKit anchors serialised inside it. Availability distinguishes a missing Health Connect app from one needing an update; macOS, other desktops and browsers report unsupported.

Plugins can now also declare manifest additions and signing requirements the CLI merges into generated hosts: `android-query-packages` for package visibility, `android-activities` for the activities and aliases a plugin's Kotlin sources need (Health Connect's permission-rationale entry points are the first user), and `apple-entitlements` the generated iOS app is signed with, dropping the entitlements file again when the last plugin needing one is deselected. The app still owns its `NSHealthShareUsageDescription` purpose string and any optional Health Connect background or history permissions it wants to request.
