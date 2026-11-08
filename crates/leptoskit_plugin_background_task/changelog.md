# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_background_task: named OS task registration

`leptoskit_plugin_background_task` registers and unregisters named background tasks, reports Available/Restricted/Unavailable, and completes an in-flight invocation. Distinct from `leptoskit_plugin_background` AppService keep-alive. Guests enable `guest` and reuse `guest::BackgroundTask`.
