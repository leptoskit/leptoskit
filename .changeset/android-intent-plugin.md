---
"leptoskit_plugin_android_intent": minor
---

# Android intents plugin

`leptoskit_plugin_android_intent` ports Flutter's `android_intent_plus`: guests describe an intent with `Intent::new().action(…).data(…).extra(…)` and launch it, open it through a titled chooser, send it as a broadcast, start a service, check whether an activity resolves it, read the package, class, and label it resolves to, or launch an `intent:` URI. Extras cover booleans, ints, longs, doubles, strings, nested bundles, byte arrays, integer and string lists, and boolean, int, long, double, and string arrays.

Only Android registers the plugin; everywhere else `guest::AndroidIntent::new` returns `Unavailable`. Requests travel as a bounded little-endian wire that the Kotlin plugin validates again, because guest code can change over the air: URI-permission grant flags and `file:` URIs are refused with `PermissionDenied`, extras can't carry parcelables, and `intent:` URIs can't name a component or selector and only reach browsable activities. A missing activity or service is `Unavailable` and an Android `SecurityException` is `PermissionDenied`. Resolution on Android 11 and later only sees apps the app's own `<queries>` declares.
