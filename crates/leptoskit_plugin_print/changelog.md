# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_print: print dialog presentation

`leptoskit_plugin_print` queries print readiness and presents the system print dialog for HTML or a PDF URI on iOS, Android, and macOS. Completion is dialog finish or job acceptance only, never proof of paper output. Guests enable the `guest` feature and reuse `guest::Print`.
