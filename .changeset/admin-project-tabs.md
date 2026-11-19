---
"leptoskit_admin": minor
"leptoskit_admin_pages": minor
---

# A project's tabs and actions run in Rust

A project's Overview, Changes, Releases, Operations, Backups and Config tabs now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their actions ask through a confirm sheet built on leptoskit's `Modal`, which keeps focus inside it and gives it back to the button that asked. Pressing a plan row opens the plan's Workings with the lines that cause it marked. Health is still drawn by the server for now.
