---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# Sign-in and refusal run as the pages' own Rust

The admin's sign-in page, and the page that turns away someone who isn't an admin, are now `Page::SignIn`, drawn without the shell and picked up by the admin's Wasm client. Continue and Switch account start sign-in with `leptoskit::auth::client::sign_in_with_provider` (Switch account asks with `Prompt::Login`), and a main site that can't start says why on the card.
