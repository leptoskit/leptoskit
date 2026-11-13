# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### SMS, mail composer, and dismissible web browser plugins

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #673](https://github.com/leptoskit/leptoskit/pull/673)

Three new Expo-equivalent runtime plugins cover composer and browsing presentation that were still missing:

- **`sms`** queries composer availability and presents the system SMS sheet with optional recipients and body. Completion is the OS callback only, never delivery. Outbound `sms:` opens stay in `linking`.
- **`mail-composer`** queries availability and presents the system mail sheet with to/cc/bcc/subject/body. Attachments are deferred in this MVP. Outbound `mailto:` opens stay in `linking`.
- **`web-browser`** opens HTTP(S) in Safari View Controller / Custom Tabs and answers on dismiss. It is distinct from `web-auth` (no OAuth return address) and from `linking.open` (which does not wait for dismiss).

Each plugin ships guest clients, wire codecs, Apple/Android backends (with unavailable stubs where a platform has no composer or dismissible sheet), docs, and tests. Desktop hosts mark the plugins unavailable via metadata.

### Fixes

- **Android plugins compile against the public SDK again.** Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #738](https://github.com/leptoskit/leptoskit/pull/738)

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_web_browser: dismissible in-app browsing

`leptoskit_plugin_web_browser` opens HTTP(S) in Safari View Controller / Custom Tabs and answers on dismiss. It is distinct from `web-auth` and `linking.open`. Guests enable the `guest` feature and reuse `guest::WebBrowser`.
