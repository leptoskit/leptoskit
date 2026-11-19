---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# Dashboards and actors run as the pages' own Rust

A dashboard, the people behind a point of a chart, and the pages an analytics screen shows instead of its data (connect the service, the service didn't answer, waiting for the first event) are now pages the admin's Wasm client picks up (`Page::Dashboard`, `Page::Actors`, `Page::Notice`). A dashboard and a page waiting for its first event read themselves again in place when the analytics stream has newer events, at most every ten seconds; the waiting page turns into its builder once events exist.
