---
"leptoskit_admin": minor
"leptoskit_admin_pages": minor
---

# The admin's Learn hub and lessons run in Rust

The Learn hub and every lesson page now come from `leptoskit_admin_pages` and hydrate in the browser, like the Glossary. They load no `admin.mjs`. Each lesson is still checked against your project's real state on the server, and the page shows the result.
