---
"leptoskit_platform_harmonyos": patch
---

HarmonyOS text uses a named line-height value instead of an inline object. ArkTS treated `{ relative?: number }` as `any` and refused to compile the page. A relative height is still a multiple of the font size, and a fixed height is still in virtual pixels.
