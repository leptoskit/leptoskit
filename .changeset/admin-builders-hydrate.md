---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# The analytics builders run as the pages' own Rust

Trends, Funnels, Retention and a saved insight are now pages the admin's Wasm client picks up. A builder control changes the page in place: the client asks the same address for the page's model (`x-admin-page: json`), keeps focus and the open Workings drawer where they were, marks the code lines the change changed, says which in the drawer, and scrolls to the first; Back and Forward work the same way. The `+ Add series`, `Breakdown` and `Change` menus are `leptoskit::aria::Menu`s. Save and Add to dashboard ask through the confirm sheet, which can now start a field with a value and offer choices (`Ask::named`, `Ask::choices`, and an `Answer` for what was chosen). A builder opens with its Workings showing from `?workings=<kind>`, and the Ask tabs are a `Tabs` set. The confirm sheet now gives focus back to the control that opened it even when that control never had focus, such as after a pointer press in Safari.

The builders no longer re-read themselves as new events arrive; that comes back with Live's event stream.
