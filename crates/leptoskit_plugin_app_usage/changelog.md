# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### App usage plugin

_Owner:_ Ifiok Jr. · _Introduced in:_ [b3d35c1](https://github.com/leptoskit/leptoskit/commit/b3d35c13b10151c8231fea893f5d3d6a941ef516)

Add the `app-usage` plugin, which reads how the device has been used. Select `app-usage` in `native.plugins` and call it from a guest through `guest::AppUsage`:

- `usage_access_status()` reports whether the person has granted Usage Access, without prompting.
- `open_usage_access_settings()` opens Android's Usage Access screen so they can grant it.
- `events(range, max)` returns typed events (an app resuming or pausing, the screen turning on or off, the lock screen showing or being dismissed, startup and shutdown), oldest first and capped at `max`.
- `sessions(range)` returns per-app foreground sessions clipped to the range.
- `summary(range)` returns screen-on time, the unlock count, and per-app foreground totals.

Android reads `UsageStatsManager` and declares `PACKAGE_USAGE_STATS`. iOS and macOS give apps no per-app usage, so sessions and summaries return `Unavailable` there, and `events` waits for the lock and unlock transitions seen while the app runs. Browsers, Windows, and Linux return `Unavailable`.
