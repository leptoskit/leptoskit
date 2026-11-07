---
"release": minor
---

# Native guests get real, cancellable timers

A generated native app's guest now has a real timer. `leptoskit_bridge::guest::sleep`, re-exported as `leptoskit::guest::sleep`, waits on the host instead of resolving at once, and `leptoskit_query`'s `guest` feature uses it for retry backoff, debounce, and cache expiry. Dropping the future cancels the host's timer, so an abandoned retry or debounced update costs nothing until its deadline.

The timer is the reserved `leptoskit.timer` plugin on the ordinary bridge, not a new mandatory Wasm import, so a bundle still loads on an older host. That host leaves the plugin unregistered and reports `Unavailable`; the query cache's `has_timer` reports `false` there rather than pretending a sleep finished. A custom provider installed with `install_sleep` still takes precedence, and browser builds keep using `setTimeout`.

Generated macOS, iOS, Android, Windows, Linux, terminal, and experimental HarmonyOS apps enable the timer. HarmonyOS schedules one ArkTS timeout for the next host deadline and cancels it when the runtime closes. Hosts bound the timer to 256 waiting or undelivered sleeps and reject a single bridge request longer than 24 hours. Query timers split longer waits into bounded requests. The new `try_sleep` operation reports unavailable or rejected timers: request retries surface their original error, intervals stop, and cache cleanup retains entries when a wait cannot finish. The infallible `sleep` never reports a failed wait as elapsed; it stays pending until dropped.

Live RPC connections also wait after a failed handshake. If a host cannot schedule the retry, the connection becomes offline until new work or credentials request another attempt; missing or full timer queues no longer trigger automatic retry loops.
