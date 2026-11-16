---
"leptoskit_config": minor
---

`[native.android] push` opts the Android app into FCM push enrollment. It is a boolean, off by default: an app without it generates byte-for-byte as before. With it, Android generation adds the pinned `com.google.firebase:firebase-messaging` dependency, writes `.leptoskit/android/gradle-plugins.txt` with Google's services plugin, generates the messaging service that forwards FCM token refreshes to the notifications plugin, and declares that service in the manifest. Push on also requires the `notifications` plugin in `[native] plugins`, the app author's `google-services.json` at `hosts/android/app/google-services.json` — never generated or committed by leptoskit — and an app Gradle script that reads the generated plugins file (the current `leptoskit new` scaffold does; older apps get the exact lines from the generation error). The configuration schema carries the new key.
