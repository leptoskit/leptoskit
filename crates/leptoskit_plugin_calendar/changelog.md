# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_calendar: permission, calendars, and bounded events

`leptoskit_plugin_calendar` queries calendar permission, lists at most 64 calendars, creates events, and reads events by id or in a bounded time window (max 64). Guests enable the `guest` feature and reuse `guest::Calendar`.
