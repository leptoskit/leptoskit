---
"release": minor
---

# Platform extension crates for HarmonyOS and Meta Quest

Unsupported and experimental OS shells no longer have to grow as endless first-party backends inside the CLI. Shared contracts live in `leptoskit_platform`. HarmonyOS ArkUI host templates move to `leptoskit_platform_harmonyos`. Meta Quest gets experimental stubs in `leptoskit_platform_oculus` (`oculus` in `app.platforms`, `leptoskit new --platform oculus`, `native prepare oculus`). Packaging for Quest is not implemented yet; HarmonyOS prepare/build behaviour is unchanged for authors.
