---
"release": patch
---

# Commonplace and Huddle run on Linux

The Commonplace and Huddle examples now target Linux too. `leptoskit build --platform linux` builds each one's GTK app from its new `linux/` runner. The apps reach their server through the HTTP plugin and keep the session in the desktop's keyring, and Huddle's chat goes live over the WebSocket plugin. Along with file_routes, observatory, postcard, and the gallery, every example with a native app now runs on Linux.
