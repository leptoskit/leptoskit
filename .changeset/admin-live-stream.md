---
"leptoskit_admin": patch
"leptoskit_admin_pages": minor
---

# Live, the pulse and the builders follow an rpc stream

The Live page is now a page the admin's Wasm client picks up. The admin's analytics stream is an rpc stream, `ops::analytics_live`, served on the admin's own live connection beside an operation's progress: it sends the pulse about once a second and, on Live, each event as it arrives. Live adds each event at the top (keeping 200), holds them while paused and shows how many wait on `Resume (n)`, narrows the list by name as you type, and opens a row to its properties. The rate card and the top bar's pulse follow the stream on every page. The builders read themselves again when there are newer events, at most every ten seconds, unless a menu, a row, the confirm sheet or a field is open.

`model::Pulse` gains `per_minute`, `reason` and `latest`; all three default when absent.
