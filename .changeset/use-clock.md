---
"leptoskit": minor
"leptoskit_use": minor
---

# Add `use_clock`, a clock that advances by itself

`leptoskit_use::use_clock(interval)` returns a `Clock` signal holding the time since it was called, updated about once per `interval` on the platform's own timer: `setTimeout` in the browser, the timer plugin in a native app, and tokio or a timer thread elsewhere. Pass it to `use_timeout_fn`, `use_interval_fn`, `signal_debounced` and the other hooks that take a clock, or read it in a `Text` closure for a countdown. Until now a host had to advance a clock signal itself.

The time is measured from the wall clock and never goes back, so a tab that slept catches up in one step. The timer stops when the owner the clock was made under is cleaned up.

It is behind a new `clock` feature, because it is the one hook in `leptoskit_use` that needs a task executor and depends on `leptoskit_query`. In `leptoskit`, the `clock` feature turns on `use` and `query` too.
