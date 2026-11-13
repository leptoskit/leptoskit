# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Fixes

#### SMS, mail composer, and dismissible web browser plugins

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #673](https://github.com/leptoskit/leptoskit/pull/673)

Three new Expo-equivalent runtime plugins cover composer and browsing presentation that were still missing:

- **`sms`** queries composer availability and presents the system SMS sheet with optional recipients and body. Completion is the OS callback only, never delivery. Outbound `sms:` opens stay in `linking`.
- **`mail-composer`** queries availability and presents the system mail sheet with to/cc/bcc/subject/body. Attachments are deferred in this MVP. Outbound `mailto:` opens stay in `linking`.
- **`web-browser`** opens HTTP(S) in Safari View Controller / Custom Tabs and answers on dismiss. It is distinct from `web-auth` (no OAuth return address) and from `linking.open` (which does not wait for dismiss).

Each plugin ships guest clients, wire codecs, Apple/Android backends (with unavailable stubs where a platform has no composer or dismissible sheet), docs, and tests. Desktop hosts mark the plugins unavailable via metadata.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_linking: external links and settings

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_linking` is the leptoskit plugin for validated external HTTP(S), email, phone, and SMS links plus application settings, without executing shell text.

Add `leptoskit_plugin_linking` as a direct dependency and select the `linking` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Linking` rather than resolving the plugin on every call. Incoming deep links need host lifecycle integration and are not handled by this crate.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
