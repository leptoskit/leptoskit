---
"leptoskit_plugin_device": minor
"leptoskit_plugin_application": minor
---

# Device and package details, as in device_info_plus

`leptoskit_plugin_device` now reports what Flutter's `device_info_plus` does. Every platform adds a model identifier (`iPhone15,2`, `Mac14,2`), kernel release, available memory, data-volume size and free space, and CPU count, and a new `Snapshot::platform` section carries one platform's own details: Android `Build` fields (SDK and preview SDK level, security patch, fingerprint, `Build.ID`, board, ABI lists, sorted system features, low-RAM class), Apple flags (iOS app on Mac or Vision, Mac Catalyst, OS build, CPU frequency, localized model), Windows release registry values, Linux `os-release` fields, and browser client-hint brands. `web::high_entropy_values()` asks the browser for its detailed hints only when you call it.

Persistent identifiers stay out of the snapshot. The new `Device::identifiers()` call returns the user-assigned device name, the iOS vendor identifier and the desktop machine identifier, each `None` where the platform withholds it; on iOS 16 and later the device name needs Apple's user-assigned-device-name entitlement.

`leptoskit_plugin_application` now reports `package_info_plus`'s remaining fields: the Android signing certificate's SHA-256 digest, the installer store (the installing package on Android; App Store, TestFlight, development or simulator on iOS, guessed from the receipt), and install and update times. `executable_times()` gives Rust hosts the executable's file times.

Both snapshots move to wire version 2. Decoders still read version 1 replies from older native hosts, but a guest built before this release cannot read a version 2 reply, so ship guests built against this version with the updated host. `Snapshot` gained public fields, so code that builds one with a struct literal needs `..Default::default()`.
