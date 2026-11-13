# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Service workers and stronger extension messaging

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #584](https://github.com/leptoskit/leptoskit/pull/584)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_service_worker: page-side service worker APIs

Register a worker script, wait for control, exchange messages, open Cache Storage, describe caching strategies, and manage Web Push subscriptions. Outside a browser every call returns `Unavailable`.
