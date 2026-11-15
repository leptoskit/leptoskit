---
"leptoskit_service_worker": patch
---

The crate compiles for `wasm32` again: the crate root's `Listener` re-export was illegal on that target, where the item comes from the browser module rather than the native fallback, and an extension-style listener helper nothing called is gone. `Registration::wait_until_active` waits for a registration's own worker to activate — unlike `ready`, which waits for the page's — and errors when the install fails or never settles instead of hanging. `PushSubscription` reports the application server key its subscription was created for, and `PushManager::unsubscribe_endpoint` unsubscribes a registration's current subscription only when it is one named endpoint, so whatever replaced it is never destroyed.
