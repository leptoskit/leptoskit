# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

#### leptoskit_plugin_app_integrity: `leptoskit_plugin_app_integrity` queries attestation capability and requests a bounded opaque token for an app-owned challenge. Guests enable `guest` and reuse `guest::AppIntegrity`. App Attest / Play Integrity wiring remain later scope; hosts return Unavailable without inventing a pass.
