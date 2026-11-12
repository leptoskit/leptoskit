---
"release": patch
---

Fix the Apple device-keys signing failure path to bridge a retained Core Foundation error explicitly before encoding its status. This avoids a Swift compiler diagnostic failure when building native hosts that use device keys.
