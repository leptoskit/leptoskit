# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Fixes

#### Plugin options the app would crash on stop the build instead

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #729](https://github.com/leptoskit/leptoskit/pull/729)

An Android app whose `leptoskit.toml` had an empty `[native.plugin-options.websocket]` built, installed, and then stopped at launch with `NativeApplicationInitializationFailure: register plugin: InvalidArgument`. The WebSocket plugin can't start without `origins`, the build only checked that the table was there, and the error didn't say which plugin had failed.

`leptoskit build` now stops with the plugin and the option it needs:

```
plugin `websocket` needs `origins` in [native.plugin-options.websocket]: without it, the app stops at launch when the plugin starts
```

An async plugin lists the options its factory can't start without in its runtime metadata, and an empty string, list, or table counts as left out:

```toml
[package.metadata.leptoskit.runtime]
name = "websocket"
rust-factory = "from_config"
rust-async = true
required-options = ["origins"]
```

`http` and `websocket` require `origins`, `shell` requires `commands`, and `xr` requires nothing. Each crate's tests hold its factory to its list.

When a plugin still fails to start, the error names it: ``register plugin `websocket`: InvalidArgument``. Generated registrants install every plugin through the new `App::register`, and a host directory that can't be used is reported as `app context: …` rather than as a plugin's failure.

The `shell` plugin is now left out of iOS, Android, and HarmonyOS builds, where it can't start processes and its factory always failed, which also stopped the app at launch.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_websocket: WebSocket connections

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_websocket` is the leptoskit plugin for scoped WebSocket connections with bounded messages and explicit lifecycle.

Add `leptoskit_plugin_websocket` as a direct dependency and select the `websocket` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native connections use tokio-tungstenite with rustls and need a Tokio runtime; browser connections own their socket and event callbacks. Every destination origin is explicit.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
