---
"leptoskit_native": patch
---

# The web launch screen lifts once the app is drawn

The HTML splash that `[app.launch_screen]` injects into `index.html` used to stay over the app for good: nothing called its `leptoskitDismissSplash()`. The web renderer now fades it out once the app's first render is painted, as iOS and Android end theirs at the first frame. An app that calls `leptoskit::splash::hold()` keeps it until `leptoskit::splash::dismiss()`, which now lifts it on the web too.
