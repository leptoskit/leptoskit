---
"leptoskit_bridge_ffi": patch
"leptoskit_plugin_android_intent": patch
---

A drawing-scaled vector stroke follows the layout frame, not the view after it has been snapped out to whole pixels. At 420 dpi a 10dp edge becomes 27px, and the stroke was a little wider than the layout asked for.

`parse_and_launch` no longer opens an `intent:` URI that has no scheme. API 35's Settings activity is browsable, so an action-only Settings URI used to start it. The activity it does open still has to declare `CATEGORY_BROWSABLE` on the filter that matched.
