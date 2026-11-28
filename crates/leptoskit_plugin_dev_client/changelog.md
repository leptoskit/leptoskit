# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **`leptoskit dev` serves phone guests on the local network and prints a QR code for them.** The LeptosKit app scans that code, or takes the address, and runs the unsigned debug guest. Only a local `http` or `https` address is accepted. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)

### Fixes

- **Expose the Apple development-session callback from the shared bridge so the development-client plugin can compile independently of the app host.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
