---
"release": minor
---

# Native guests get real, cancellable timers

A generated native app's guest now has a real timer. `leptoskit_bridge::guest::sleep`, re-exported as `leptoskit::guest::sleep`, waits on the host instead of resolving at once, and `leptoskit_query`'s `guest` feature uses it for retry backoff, debounce, and cache expiry. Dropping the future cancels the host's timer, so an abandoned retry or debounced update costs nothing until its deadline.

The timer is the reserved `leptoskit.timer` plugin on the ordinary bridge, not a new mandatory Wasm import, so a bundle still loads on an older host. That host leaves the plugin unregistered and reports `Unavailable`; the query cache's `has_timer` reports `false` there rather than pretending a sleep finished. A custom provider installed with `install_sleep` still takes precedence, and browser builds keep using `setTimeout`.

Generated macOS, iOS, Android, Windows, Linux, and terminal apps enable the timer; experimental HarmonyOS does not wire it yet. Hosts bound the timer to 256 waiting sleeps and reject a single sleep longer than 24 hours.
