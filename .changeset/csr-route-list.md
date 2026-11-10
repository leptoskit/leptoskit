---
"leptoskit_router_codegen": patch
---

# Generated routes also compile in private client modules

The generated `axum_route_list()` helper is optional in client-rendered apps. An unused helper inside a private routes module no longer fails a warnings-denied build. Its documentation names the Axum method without requiring the app to depend directly on `leptos_axum`, so client-only documentation builds work too. SSR hosts can still call the same helper.
