---
"leptoskit_query": minor
---

# A fallible wait for hosts without a timer

`leptoskit_query::try_sleep` reports `SleepError::Unavailable` or `SleepError::Rejected` instead of pretending a wait elapsed. Generated sign-in screens use it for a rate-limit countdown, so on a host with no usable timer the button becomes a manual retry, the server's "try again in …" message stays visible, and the screen never claims time that did not pass. The server remains the authority on when the window opens.

This is the compatibility surface for the native guest timer work. When that timer lands, its host-backed `try_sleep` implementation replaces the fallback here without changing the screen's behavior.
