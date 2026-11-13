# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **`leptoskit dev` serves phone guests on the local network and prints a QR code for them.** The LeptosKit app scans that code, or takes the address, and runs the unsigned debug guest. Only a local `http` or `https` address is accepted. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)

### Fixes

- **Expose the Apple development-session callback from the shared bridge so the development-client plugin can compile independently of the app host.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
