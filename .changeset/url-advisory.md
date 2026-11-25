---
"release": patch
---

# `url` is 2.5.9

`url` moves from 2.5.8 to 2.5.9 across the workspace, and takes `form_urlencoded` from 1.2.2 to 1.2.3 with it, because `url` 2.5.9 requires that version. Before 2.5.9, `Url::origin()` handled a `blob:` URL by parsing its path as another URL and calling itself on the result, so a nested `blob:` URL overflowed the stack and aborted the process (RUSTSEC-2026-0592). Server rendering through `leptos_axum` makes that call for every page it renders, on an address it rebuilds from the request's `Host` and `X-Forwarded-Proto` headers. Until this update, one request with a crafted `X-Forwarded-Proto` header could crash the process of a server-rendered app it reached directly. Nothing in leptoskit's own API changed.
