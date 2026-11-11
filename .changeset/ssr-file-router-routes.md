---
"leptoskit_router": patch
"leptoskit_cli": patch
---

SSR apps now register every file route with `leptos_axum`, and the server render reads the request URL instead of always opening on Home.
