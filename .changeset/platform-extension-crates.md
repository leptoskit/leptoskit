---
"release": minor
---

# Platform extension crates for HarmonyOS

Unsupported and experimental OS shells no longer have to grow as endless first-party backends inside the CLI. Shared contracts live in `leptoskit_platform`. HarmonyOS ArkUI host templates move to `leptoskit_platform_harmonyos`. Meta Quest remains the first-party `quest` destination from the Android host (`[native.quest]`); packaging stubs are not delivered as a separate `oculus` platform-extension crate in this change.
