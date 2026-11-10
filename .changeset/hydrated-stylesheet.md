---
"leptoskit_native": patch
---

Hydrated web screens refresh their stylesheet when classes change. Switching a theme or opening a panel could introduce new style scopes while leaving the initial CSS in place, so controls lost their layout and colours. The browser regression hydrates server HTML and changes padding in both directions.
