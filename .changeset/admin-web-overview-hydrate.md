---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# The web overview runs as the pages' own Rust

The web overview is now a page the admin's Wasm client picks up (`Page::Web`). Its range, compare and filter chips and its table rows change it in place, the way a builder's controls do, and it reads itself again as events arrive. Add to dashboard asks through the confirm sheet.
