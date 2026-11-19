---
"leptoskit_admin": minor
"leptoskit_admin_pages": minor
---

# Library, People and a person's page run in Rust

The analytics Library, the People list and each person's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Search and filter build the list's address in Rust. Export asks first, and its confirm button is a download link, so the browser downloads the server's attachment itself. Delete asks for the person's name and keeps its button disabled until the name matches.
