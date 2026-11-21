# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Service workers and stronger extension messaging

_Owner:_ Ifiok Jr. · _Introduced in:_ [363ebad](https://github.com/leptoskit/leptoskit/commit/363ebadd2effe51d9b954c31ab0b6bcbfdc46ab8)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

### Fixes

- **Disable only the authenticated person's service enrollment, preserving the browser subscription and permission for reuse.** Require a successful revoke acknowledgement with a stable retry key; never infer success from an endpoint list. Remove the unused endpoint-matching unsubscribe helper. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **The crate compiles for `wasm32` again: the crate root's `Listener` re-export was illegal on that target, where the item comes from the browser module rather than the native fallback, and an extension-style listener helper nothing called is gone.** `Registration::wait_until_active` waits for a registration's own worker to activate — unlike `ready`, which waits for the page's — and errors when the install fails or never settles instead of hanging. `PushSubscription` reports the application server key its subscription was created for, and `PushManager::unsubscribe_endpoint` unsubscribes a registration's current subscription only when it is one named endpoint, so whatever replaced it is never destroyed. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_service_worker: page-side service worker APIs

Register a worker script, wait for control, exchange messages, open Cache Storage, describe caching strategies, and manage Web Push subscriptions. Outside a browser every call returns `Unavailable`.
