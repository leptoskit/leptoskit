# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-08)

### Features

#### leptoskit_plugin_cellular: carrier and radio snapshot

`leptoskit_plugin_cellular` is the leptoskit plugin for carrier name, MCC/MNC, radio generation, and VoIP allowance snapshots. Unknown, Unavailable, and Restricted stay explicit; values are never invented. iOS uses CoreTelephony, Android uses permission-light TelephonyManager fields, desktop and the browser answer Unavailable, and camera-style waits are out of scope for this MVP.
