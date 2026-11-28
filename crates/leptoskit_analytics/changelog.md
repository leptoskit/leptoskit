# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **The insights engine (§10 of the analytics service design), in its first form.** A new `insights` feature holds the query types — trends, funnels, retention, the web overview, the events list, and the people behind any of their points — as plain serde values an app or an admin compiles without the service. With the `service` feature, the `run` operation plans each query into SQL for SQLite or Postgres and answers with the series, steps, grid, or page beside the SQL it ran, its bound parameters, and its timing (the Workings drawer's data). Property names travel as bound parameters and session columns come from an allow-list, so a name carrying SQL reads a property of that name instead of running it; every query resolves persons through `person_overrides`, so a merge applies to past events at once. Saved insights and dashboards are app-only CRUD over new `insights` and `dashboards` tables, with the default "Web overview" available as data. Live refresh: the batch writer bumps one live `ingest_ticks` row at most once per `refresh_seconds` (default 10, configurable), so watched insights re-run as events arrive without the events table being live. A minimal `live_events` stream tails the writer's last 10,000 events, filtered server-side and resumable, each item with the matched rate. The service's `routes` now also returns its live socket, which the binary starts. All query operations answer to the app's caller, like the person operations before them. _Owner:_ Ifiok Jr. · _Introduced in:_ [c7f6ab9](https://github.com/leptoskit/leptoskit/commit/c7f6ab90ecc33b4f84b76cd79b40e8e322cd3071)

#### Persons, groups, and sessions at the analytics service, and views the browser records on its own.

_Owner:_ Ifiok Jr. · _Introduced in:_ [93b1c47](https://github.com/leptoskit/leptoskit/commit/93b1c4715d67ee50c7cf1da9d84baad94f6b2561)

The service now builds a person for every identified id and joins a visitor's made-up id to their real one when the app identifies them, so what someone did before and after signing in is one story. Person properties keep the value of the most recent event per property, whatever order events arrived in — an offline phone's late batch no longer overwrites what you learned later. Merging two already-identified persons is refused and counted, never guessed. Each visit becomes one session row with its entry and exit pages, duration, bounce, and how it arrived: the channel from the referrer and UTM parameters, and the device, browser, and operating system parsed from the user agent at the service, so clients stay small. The browser's client now records a `$pageview` on load and on history navigation and a `$pageleave` when the page goes, with the session's first view carrying its UTM parameters and referrer; `Options::pageviews` turns that off.

Your app's server can read and manage persons — list them, read one with their events and sessions, delete one and everything stored about them, or export one as JSON — and the service's `person_profiles` setting decides who gets a person at all.

The core browser client grows from 11,985 to 13,584 bytes for the automatic views, so its size budget changes from 12 KiB to 14 KiB, decided in the open as the analytics design's size rule requires.

#### Retention cohorts every period; event definitions

_Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)

- **Retention cohorts.** A retention query now answers one cohort for every period in its range, oldest first, even when nobody started in a period (its size is 0). `ActorPoint::RetentionCell { start_period, .. }` therefore indexes `cohorts` directly. Each cohort is followed for `periods` periods or until the range ends, so later cohorts have shorter `returns`. First-time cohorts look at the start event's whole history, so someone who first did it before the range doesn't count as new inside it.
- **Wire rename.** A retention query's first-time or recurring choice now travels as `retention_kind`. Before, it travelled as `kind`, which collided with `Query`'s own tag: saving a retention insight failed.
- **`event_definitions`.** This app-only operation lists every event name the service has seen, with:
  - its volume
  - when it was first and last seen
  - the fields of its registered Rust type, if it has one
  - the properties its recent events carried

  The admin's builders read it.

### Fixes

- **Preserve typed fallback context and queue bounds.** Keyed retries of unknown typed schemas retain their original context, UUID and field precedence. Entries expanded past a batch or queue byte limit are dropped and counted, and total queue accounting includes retained contexts. Catalogue validation also checks known vector element types recursively. _Owner:_ Ifiok Jr. · _Introduced in:_ [807eb2f](https://github.com/leptoskit/leptoskit/commit/807eb2f61bfa8b3bd57a680ebcbf6443c7746ce5)
