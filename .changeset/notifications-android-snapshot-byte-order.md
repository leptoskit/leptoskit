---
"leptoskit_plugin_notifications": patch
---

# Android push registration replies in the right byte order

On Android the plugin wrote a remote registration's revision, state and string length big-endian, while every other reply and the Rust side that reads it are little-endian. An app asking for its push token read a state of `50331648` instead of `3`, or a length of hundreds of megabytes, and failed to decode the reply. The snapshot is now written little-endian like the rest.
