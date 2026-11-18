---
"leptoskit_plugin_background": patch
---

# Background-service checks in the location and recorder plugins now see the running service

On Android, the location and recorder plugins check that the `background` plugin's foreground service runs with the right type before they start in the background. They read that state by name, and it was never visible to them, so the check always failed and background location updates and background recording were refused even with the service running. The state is now a public field that release builds keep through R8.
