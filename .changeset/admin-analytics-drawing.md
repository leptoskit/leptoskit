---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# The analytics charts and builders move to the pages crate

`leptoskit_admin_pages::analytics` now holds the analytics area's drawing: number and date formatting, a query as an address (`params`), the trend, funnel and retention charts, and each builder's controls. The admin's server still runs the queries and draws these pages itself for now; the move lets the next change hydrate them. Nothing on screen changes.
