# Research: Datadog patterns + columnar storage options for a Rust self-hosted analytics service

Researched 2026-11-07. Notes are labelled by confidence:

- **[V]**: verified against the cited page or source code during this research.
- **[U]**: unverified (from memory, a secondary source, or an inference). Check before relying on it.
- Unlabelled bullets are design commentary.

Crate versions come from the crates.io API on 2026-11-07.

---

## PART A: DATADOG

### A1. Log Management: Live Tail

Source: https://docs.datadoghq.com/logs/explorer/live_tail/

- [V] Live Tail is a near real-time stream of **indexed and non-indexed logs**. It reads the post-pipeline stream before indexing, so logs dropped by index exclusion filters still appear.
- [V] Logs in Live Tail are already "structured, processed, and enriched by Log Pipelines". Pipelines run before the tail, and indexing runs after it.
- [V] You open it from the Log Explorer time picker ("Live Tail" option). It uses the normal Log Search Syntax.
- [V] **Sampling:** when too many matching logs flow in, output is sampled. The docs call the sampling "uniformly random", so the stream stays statistically representative of throughput. Their advice for seeing every log is to narrow the query with more filters.
- [V] **Full-text search (`*:foo`) is not allowed in Live Tail.** It is also rejected in index, archive, pipeline, rehydration and log-to-metric filters. Live Tail supports only free-text search over message-like fields plus attribute and tag filters, and CIDR() works there. (https://docs.datadoghq.com/logs/explorer/search_syntax/)
- [U] UI behaviour from memory, not on the current doc page:
  - the stream pauses automatically when the user scrolls or hovers, and resumes with a button;
  - the browser keeps a bounded ring buffer of recent rows;
  - side-panel facets (service, status, host) give one-click filters;
  - you can pick display columns and switch between raw and JSON views.
- [U] Datadog publishes no display rate cap. For comparison, Lumigo's Live Tail caps at 50 new logs/s and samples above that (https://docs.lumigo.io/docs/logs-live-tail). Exabeam has explicit Pause/Play.
- Design takeaways for our event live tail:
  - (a) tail the stream after enrichment and before sampling/retention, so dropped events stay visible live;
  - (b) evaluate the filter server-side and push only matches;
  - (c) cap the client rate (e.g. 50 to 100 rows/s) with uniform random sampling, and report the sample rate in the UI;
  - (d) pause buffers client-side or stops the push;
  - (e) restrict the live filter language to cheap predicates (no full-text, no regex over all attributes).

### A2. Logging without Limits (ingest vs index)

Sources: https://docs.datadoghq.com/logs/guide/getting-started-lwl/, https://docs.datadoghq.com/logs/log_configuration/archives/, https://docs.datadoghq.com/logs/log_configuration/rehydrating/, https://docs.datadoghq.com/logs/log_configuration/flex_logs/

- [V] Ingestion is decoupled from indexing. Everything is ingested and processed; only a subset is indexed (made searchable and billed per indexed event).
- [V] **Exclusion filters** are attached to an index.
  - Each filter is a query plus an exclusion percentage (sampling).
  - The first matching filter wins.
  - Filters can be disabled or edited at any time.
  - Excluded logs are "**only** excluded from indexing". They are still visible in Live Tail, still archived, and still counted by log-based metrics.
- [V] **Generate metrics from logs** runs at ingest, so it counts excluded logs too. Datadog warns against grouping by high-cardinality attributes such as user id or request id.
- [V] **Archives:**
  - Format is compressed JSON, zstd by default (`.json.zst`) with gzip optional. Changing the format affects only new files.
  - Path layout: `/<prefix>/dt=<YYYYMMDD>/hour=<HH>/archive_<HHmmss.SSSS>.<UUID>.json.gz` (Hive-style partitions).
  - Destinations are S3, GCS and Azure.
  - Upload frequency "can vary"; the docs say to check the bucket after about 15 minutes.
  - Archived logs include tags by default, which makes the files larger.
- [V] **Rehydration ("historical views"):**
  - Up to 1B events per historical view, and admins can set a lower default cap.
  - The query is applied after the files are downloaded, so cost scales with the time range scanned, not with the matches. Narrowing the date range is the main lever.
  - Rehydrated logs stay searchable for the chosen retention: 15 days by default, with options of 3, 7, 15, 30, 45, 60, 90 or 180 days.
  - Both scanned and indexed logs are billed.
  - [V] Datadog now steers users to **Archive Search**, which streams results from the archive without re-indexing and bills only the data scanned.
- [V] **Flex Logs** is a cheaper storage tier with separately sized query compute.
  - Retention is 30 to 450 days on scalable compute, or 3, 6, 12 or 15 months on Starter.
  - Compute sizes range from Starter (<10B stored events) through XS, XS+, S, M and L (500B to 1T events). Each size roughly doubles the previous one.
  - Monitors don't run on Flex. When compute is exhausted, queries retry and can fail.
  - Logs can move from Standard to Flex (for example, 3 days Standard then 87 days Flex).
- Design takeaways:
  - three tiers: hot indexed (DB) → cold archive (Parquet or JSONL.zst on object storage, `dt=/hour=` partitions) → on-demand scan;
  - sampling and exclusion rules decide what is indexed;
  - metrics are computed on the full stream at ingest.

### A3. Pipelines and processors

Source: https://docs.datadoghq.com/logs/log_configuration/processors/, https://docs.datadoghq.com/logs/log_configuration/parsing/

- [V] The 21 processor types:
  - parsing: Grok Parser, URL Parser, User-Agent Parser, GeoIP Parser, Decoder (Base64/Hex);
  - remappers: Log Date, Log Message, Log Status, Service, Trace (log↔trace), Span, and Attribute (rename or move attributes and tags);
  - derived values: Category (adds an attribute when a query matches), Arithmetic (formula over numeric attributes), String Builder (template);
  - lookup and enrichment: Lookup (reference table or mapping), Threat Intel (IoC match), OCSF normalisation;
  - structure: Array, Array Map, Exclude Attribute.
- [V] Datadog recommends at most 20 processors per pipeline. This is a best practice, not a hard limit.
- [V] **Grok syntax:** `%{MATCHER:EXTRACT:FILTER}`.
  - Example: `rule %{word:user} connected on %{date("MM/dd/yyyy"):date}`.
  - The first matching rule wins, rules may reference earlier rules, and helper rules are supported.
  - A rule must match the whole line.
  - "Extract from" lets a rule parse a named attribute instead of `message`.
- [V] **Matchers:**
  - at query and ingest time: `word`, `notSpace`, `number`, `integer`, `data`;
  - at ingest only: `date(pattern[,tz[,locale]])`, `regex()`, `boolean`, the `*Str`/`*Ext` number variants, the quoted strings, `uuid`, `mac`, `ipv4`, `ipv6`, `ip`, `hostname`, `ipOrHost`, `port`.
- [V] **Filters:** `number`, `integer`, `boolean`, `nullIf`, `json`, `rubyhash`, `useragent`, `querystring`, `decodeuricomponent`, `lowercase`, `uppercase`, `keyvalue(sep,allowlist,quoting,delim)`, `xml`, `csv`, `scale(factor)`, `array(...)`, `url`.
- [V] **Sensitive Data Scanner** (https://docs.datadoghq.com/security/sensitive_data_scanner/):
  - scanning groups (a filter for what to scan) contain rules, either from a library (email, credit card, API keys...) or custom regex;
  - actions: redact (replace with a token), partial redact, hash (non-reversible), mask (reversible for users with the unmask permission; logs, APM and RUM only);
  - per-product sampling of 10 to 99% cuts scan cost, but obfuscation isn't available on sampled data;
  - it runs in the Datadog cloud before indexing, or on-prem in Observability Pipelines before data leaves the network;
  - it covers Logs, APM spans, RUM events and Events.
- Design takeaway: a small processor list covers most needs. A minimal set is attribute rename/remap, UA parse, GeoIP, URL parse, category-by-query, regex redact/hash, and drop attribute. Pipeline order is filter → processors → (live tail, metrics, archive) → index sampling.

### A4. Event Management / Events Explorer

Sources: https://docs.datadoghq.com/service_management/events/, https://docs.datadoghq.com/service_management/events/explorer/searching/, https://docs.datadoghq.com/monitors/types/event/, https://docs.datadoghq.com/events/guides/migrating_to_new_events_features/, https://docs.datadoghq.com/service_management/events/correlation/patterns

- [V] Events are discrete occurrences:
  - created automatically by monitors, Watchdog and Error Tracking;
  - collected from 100+ integrations (k8s, Docker, Jenkins, ECS), the Agent, and the Events API (deploys, change requests, 3rd-party alerts);
  - enriched and normalised by event pipelines and processors.
- [V] The legacy Event Stream and its event monitors were retired in 2022 (monitors stopped on 2022-06-30) in favour of the **Events Explorer**.
- [V] Events search uses the **logs search syntax**:
  - `@` prefix for attributes, as in `@evt.name:foo`;
  - reserved fields: `source`, `status`, `host`, `service`;
  - `status` values: emergency, alert, critical, error, warn, notice, info, debug, ok.
- [V] Events can generate metrics with 15-month retention. [U] Raw event retention is not stated on the pages read.
- [V] **Event monitors:**
  - the query is an Events Explorer search, measured as a count, a facet unique-count, or an aggregate (avg/median/min/sum) of an attribute;
  - thresholds use above, above-or-equal, below, below-or-equal, equal, or not equal;
  - group by up to 4 facets. Top-value limits: 1 facet 1000; 2 facets 30 each (900 groups); 3 facets 10 each; 4 facets 5 each;
  - simple alert or multi alert, up to 1000 groups;
  - limit of 1000 event monitors per account;
  - monitors don't alert on monitor events, to avoid loops;
  - late events are back-dated to their occurrence time, so widen the window or add an evaluation delay.
- [V] **Correlation (patterns → cases):**
  - events are deduplicated into alerts by `aggregation_key`, then alerts are correlated into a case (work item) using grouping tags/facets;
  - settings: minimum alert count to open a case; correlation window (how long new alerts can join, measured from the first alert); deduplication window; "extended dedupe" up to 30 days for unresolved alerts; auto-closure;
  - editing a pattern stops processing for live cases;
  - "Intelligent Correlation" uses ML on shared tags, attributes, trigger time and related resources.
- [V] Since 2025-03-01, the monitor-event `aggregation_key` includes the monitor group.
- Design takeaway:
  - treat "events" as typed rows that use the same query language as logs;
  - monitor = scheduled count/aggregate query with group-by limits;
  - correlation = dedup key + group key + time window.

### A5. RUM (Real User Monitoring)

Sources: https://docs.datadoghq.com/real_user_monitoring/browser/data_collected/, browser-sdk source, https://github.com/DataDog/rum-events-format

- [V] **Event types and retention:**

  | Event     | Retention | Notes                                                                                                       |
  | --------- | --------- | ----------------------------------------------------------------------------------------------------------- |
  | Session   | 30 days   | `session.id`, `session.type` user/synthetics, `is_active`, `time_spent`, `initial_view`/`last_view`, counts |
  | View      | 30 days   | one per page or route; `view.id` links its children                                                         |
  | Action    | 30 days   | click or custom; `action.type`, `action.name`, `action.target.name`, `loading_time`, `frustration.type`     |
  | Resource  | 15 days   | XHR, fetch, img, css, js; type, method, status_code, url, duration, size, provider                          |
  | Error     | 30 days   | source, type, message, stack                                                                                |
  | Long Task | 15 days   | main-thread block over 50 ms; `long_task.duration`                                                          |

- [V] **Session timeout:** `SESSION_EXPIRATION_DELAY = 15 * ONE_MINUTE` (inactivity) and `SESSION_TIME_OUT_DELAY = 4 * ONE_HOUR` (maximum length). The session id lives in a cookie (or localStorage or memory; `SessionPersistence` enum) with a one-year cookie expiry. A new session starts after 15 min idle or 4 h total. (https://github.com/DataDog/browser-sdk/blob/main/packages/browser-core/src/domain/session/sessionConstants.ts)
- [V] **View updates are versioned:**
  - the SDK re-sends view events as metrics accumulate (`type: view_update` in newer formats);
  - `_dd.document_version` is the "Version of the update of the view event", and the backend keeps the latest;
  - this maps directly onto ReplacingMergeTree(version) or an upsert keyed by `view.id`.
  - (https://github.com/DataDog/rum-events-format/blob/master/schemas/rum/_view-properties-schema.json, samples/rum-events/view_update.json)
- [V] **Browser batching:** a batch flushes on 30 s (`FLUSH_DURATION_LIMIT`), 50 messages (`MESSAGES_LIMIT`), or 16 KiB (`RECOMMENDED_REQUEST_BYTES_LIMIT`), whichever comes first. The maximum message size is 256 KiB, and pages flush on exit (page-may-exit observable). (browser-sdk `transport/flushController.ts`, `httpRequest.ts`, `batch.ts`)
- [V] **Frustration signals** (https://docs.datadoghq.com/real_user_monitoring/browser/frustration_signals/):
  - rage click: more than 3 clicks within a 1-second sliding window;
  - dead click: a click on a static element that produces no action;
  - error click: a click right before a JS error;
  - requires `trackUserInteractions: true` (SDK ≥4.14) and is mouse-only, so keypresses don't count;
  - counts appear as `session.frustration.count`, `view.frustration.count` and `action.frustration.type[]`.
- [V] **Core Web Vitals fields:**
  - LCP: `view.largest_contentful_paint`, with target selector and sub-parts load_delay, load_time, render_delay;
  - INP: `view.interaction_to_next_paint`, with target selector and sub-parts input_delay, processing_duration, presentation_delay;
  - CLS: `view.cumulative_layout_shift`, with target selector;
  - FID (legacy): `view.first_input_delay`;
  - plus FCP, dom_interactive, load_event and loading_time.
- [V] **Sampling** (https://docs.datadoghq.com/real_user_monitoring/guide/sampling-browser-plans/):
  - `sessionSampleRate` (default 100) runs first;
  - `sessionReplaySampleRate` (default 0 since v5) is a percentage of the tracked sessions;
  - example: 60/50 → 40% dropped, 30% RUM only, 30% RUM+Replay;
  - `startSessionReplayRecordingManually: true` plus `startSessionReplayRecording()` records only after some state is reached.
- [V] **RUM without Limits retention filters** (https://docs.datadoghq.com/real_user_monitoring/rum_without_limits/retention_filters/):
  - filters run on ingested events of any type and decide per **session** whether to retain it (30 days);
  - once a session is retained, its later events are kept;
  - filters are ordered; each is a RUM Explorer query plus a rate (minimum 0.1%), and exclusion filters have no rate;
  - permanent filters: RUM-APM flat 1% of sessions with traces, Synthetics sessions, and sessions with forced replay;
  - cross-product: an APM filter indexes traces for X% of the sessions that the parent filter retained.
- [V] **RUM ↔ APM linking** (https://docs.datadoghq.com/real_user_monitoring/platform/connect_rum_and_traces/):
  - `allowedTracingUrls` matches by string prefix, RegExp or function;
  - `propagatorTypes` per URL: datadog, tracecontext, b3, b3multi. The default is tracecontext+datadog;
  - headers: `x-datadog-trace-id`, `x-datadog-parent-id`, `x-datadog-origin: rum`, `x-datadog-sampling-priority`, `traceparent`, `tracestate: dd=s:<prio>;o:rum`;
  - `traceSampleRate` (default 100) is independent of session sampling;
  - `traceContextInjection: sampled|all`;
  - CORS: the backend must allow these headers in `Access-Control-Allow-Headers` and handle preflight;
  - the initial HTML document isn't traced.
- [V] **Session Replay** (https://docs.datadoghq.com/session_replay/browser/):
  - the browser recorder is built on **rrweb**: a DOM and CSS snapshot, then incremental mutations, mouse and input events with timestamps;
  - compression runs in a dedicated **web worker**; expected bandwidth is under 100 kB/min;
  - default retention is 30 days, extendable to 15 months;
  - mobile replay uses "wireframes" (rectangles with stable ids): a full snapshot, then incremental snapshots of changed wireframes. [U] The worker compression codec is deflate (from memory).
- [V] **Mobile batching and offline (iOS)**, from `DatadogCore/Sources/PerformancePreset.swift`:
  - `maxFileSize` is 5 MB ("matches the server's 5 MB per-batch limit"), `maxObjectsInFile` is 1000 (backend maximum), `maxObjectSize` is 1 MB (backend maximum per event);
  - `maxDirectorySize` is **512 MB**, and `maxFileAgeForRead` is **18 hours** (older batches are dropped);
  - `batchSize` small/medium/large sets mean file age to 3 s, 10 s or 35 s (1 s in app extensions);
  - `uploadFrequency` frequent/average/rare sets minUploadDelay to 0.5 s, 2 s or 5 s;
  - upload delay factors are initial ×5, min ×1, max ×10, with a change rate of 0.1 (adaptive backoff), plus 1 s jitter;
  - `batchProcessingLevel` (default .medium) sets the maximum number of batches per upload cycle;
  - `backgroundTasksEnabled` defaults to false (about +30 s in the background), and a `DataEncryption` hook can encrypt data on disk.
  - Source: https://github.com/DataDog/dd-sdk-ios/blob/develop/DatadogCore/Sources/PerformancePreset.swift
- [V] **Mobile batching and offline (Android)**, from `FilePersistenceConfig.kt`:
  - `MAX_BATCH_SIZE` 5 MB, `MAX_ITEM_SIZE` 1 MB, `MAX_ITEMS_PER_BATCH` 1000;
  - `OLD_FILE_THRESHOLD` **18 h**, `MAX_DISK_SPACE` **512 MB**;
  - `MAX_DELAY_BETWEEN_MESSAGES_MS` 5000 (a file is "recent" for writing), cleanup threshold 5 s;
  - backpressure: the internal queue drops new tasks after 1024 items;
  - `setBatchSize` S/M/L, `setUploadFrequency` FREQUENT/AVERAGE/RARE, `setBatchProcessingLevel` LOW/MEDIUM/HIGH;
  - consent NOT_GRANTED wipes batched data.
  - Source: https://github.com/DataDog/dd-sdk-android/blob/develop/dd-sdk-android-core/src/main/kotlin/com/datadog/android/core/internal/persistence/file/FilePersistenceConfig.kt
- [V] iOS defaults: `trackBackgroundEvents` false (enabling it can create extra sessions) and `trackFrustrations` true (error taps).
- Design takeaways:
  - copy the session state machine as is: 15 min idle, 4 h maximum, id in first-party storage;
  - view events are upserts keyed by `view.id` with `document_version`;
  - mobile clients write file-based batches (≤5 MB / ≤1000 events / ≤1 MB per event), drop batches older than 18 h, cap disk at 512 MB, and upload with adaptive backoff and jitter;
  - leptoskit natives can reuse this exactly.

### A6. Query language worth copying for a live tail filter

Source: https://docs.datadoghq.com/logs/explorer/search_syntax/

- [V] **Terms and phrases:** `hello`, `"hello world"` (straight quotes only).
- [V] **Booleans:**
  - `AND` (implicit), `OR` and `-` (NOT) are case-sensitive keywords;
  - parentheses group: `env:(prod OR test)`, `(env:prod AND -version:beta)`.
- [V] **Reserved, unprefixed fields:** `service:`, `status:`, `host:`, `source:`, `trace_id:`, `message`.
- [V] **Attributes and tags:**
  - attributes use the `@` prefix, with dotted nested paths: `@http.url_details.path:"/api/v1/test"`;
  - existence: `@attr:*`; missing: `-@attr:*`;
  - tags: `env:prod`, or `tags:<bare>` for non key:value tags.
- [V] **Wildcards:**
  - `*` matches within one word (`web*`, `*mongo`); `**` crosses spaces; `?` matches one special character or space;
  - wildcards are ignored inside quotes, and wildcard searches are slower;
  - escape special characters with `\` or quote the value.
- [V] **Numbers:**
  - comparisons `@http.response_time:>100` (also `>=`, `<`, `<=`);
  - ranges `@http.status_code:[400 TO 499]`;
  - numeric comparison requires a numeric facet, i.e. a typed column.
- [V] **Arrays:**
  - a match on any element matches: `@users.names:Peter`;
  - `@perms:(4 6)` requires all listed values; `@perms:[2 TO 6]` matches any element in range.
- [V] **IP blocks:** `CIDR(@network.client.ip, 13.0.0.0/8, ...)` and `NOT(CIDR(...))`. They work in Live Tail and support IPv4 and IPv6.
- [V] **Case and fields:**
  - attribute search is case-sensitive; use full-text or a lowercase processor for case-insensitive matching;
  - calculated fields are referenced with `#name`;
  - saved views store the query, columns, time range and facets.
- [V] **Word splitting:** free text splits on Unicode word boundaries (`hello-world` splits, `hello_world` doesn't, `192.168.1.1` stays whole).
- Other syntaxes worth comparing:
  - [U] Kibana KQL: `field:value and not x:y`, `field >= 10`, `field:*`, nested `items:{ name:x and qty>5 }`;
  - [U] Lucene: `field:[a TO b]`, `field:/regex/`, `~` fuzzy, `^` boost;
  - [U] GitHub-style `key:value` qualifiers;
  - [U] Grafana Loki LogQL: `{app="x"} |= "err" | json | status >= 500` (stream selector plus pipeline stages);
  - [U] PostHog's HogQL is SQL.
- Recommended subset for our live tail:
  - `term`, `"phrase"`, `key:value`, `@prop.path:value`, `-` negation, AND/OR with parentheses;
  - `:>`, `:>=`, `[a TO b]`, `*` prefix/suffix wildcard, `key:*` existence, `CIDR()` optional;
  - compile to a predicate AST that is (a) evaluated in-process on the live stream and (b) translated to SQL WHERE (SQLite json_extract, Postgres `->>`, ClickHouse JSONExtract, or DataFusion) for history;
  - no full-text on the live path.

---

## PART B: COLUMNAR STORAGE FROM RUST

Crate versions as of 2026-11-07:

| Crate                        | Version                  | Released |
| ---------------------------- | ------------------------ | -------- |
| datafusion                   | 55.2.0                   | 2026-10  |
| arrow / parquet              | 60.1.0                   | 2026-10  |
| object_store                 | 0.14.2                   |          |
| deltalake                    | 1.1.2                    |          |
| iceberg / iceberg-datafusion | 0.11.0                   | 2026-11  |
| duckdb                       | 1.20000.1 (DuckDB 2.0.0) |          |
| clickhouse                   | 0.15.3                   |          |
| klickhouse                   | 0.16.0                   |          |
| rusqlite                     | 0.41.1                   |          |
| sqlx                         | 0.9.0                    |          |
| tantivy                      | 0.27.0                   |          |
| lance                        | 14.0.2                   |          |
| vortex-array                 | 0.93.0                   |          |

### B1. ClickHouse

- **Rust clients:**
  - [V] `clickhouse` (https://github.com/ClickHouse/clickhouse-rs) is the official pure-Rust typed client, licensed Apache-2.0/MIT:
    - HTTP transport with `RowBinaryWithNamesAndTypes` and schema validation by default; `with_validation(false)` switches to plain RowBinary;
    - streaming `insert()` commits on `end()`;
    - an `inserter` feature auto-commits by row, byte or time thresholds;
    - LZ4 by default, with LZ4HC and ZSTD; TLS via native-tls or rustls; serde `Row` derive;
    - mocks via `test-util`;
    - no native TCP protocol yet, no `Dynamic` type, JSON only as a string (CH 24.10+);
    - an insert is atomic only within one partition and under `max_insert_block_size`;
    - MSRV 1.89 for 0.14.x.
  - [V] `klickhouse` (https://github.com/Protryon/klickhouse, Apache-2.0, about 130 stars, pushed 2026-11) uses the **native TCP** protocol. Version 0.16.0. [U] It also supports derive rows and migrations.
  - [V] `chdb-rust` (embedded ClickHouse, like DuckDB-in-process) is labelled "experimental and unstable" and needs libchdb (https://github.com/chdb-io/chdb-rust).
- **Engines:**
  - [V] ReplacingMergeTree(ver[, is_deleted]) (https://clickhouse.com/docs/guides/replacing-merge-tree):
    - rows with equal ORDER BY keys are deduplicated to the highest version during background merges, so deduplication is **eventual**;
    - use `FINAL` for correct reads; filtering on key columns keeps FINAL cheap;
    - partitioning by an immutable key plus `do_not_merge_across_partitions_select_final=1` speeds up FINAL;
    - is_deleted rows are purged only via `OPTIMIZE ... FINAL CLEANUP` (experimental setting);
    - recommended for fewer than 10% deletes.
  - [V] **PostHog persons:**
    - `person` uses ReplacingMergeTree(ver=version);
    - `person_distinct_id2(team_id, distinct_id, person_id, is_deleted Int8, version Int64)` uses ReplacingMergeTree(version) ORDER BY (team_id, distinct_id);
    - reads use `argMax(person_id, version) ... HAVING argMax(is_deleted, version)=0`;
    - the earlier `person_distinct_id` used **CollapsingMergeTree** (_sign) and was superseded;
    - the later "persons on events" work denormalises person_id and properties onto events at ingest.
    - Sources: https://posthog.com/handbook/engineering/clickhouse/schema/person-distinct-id, https://github.com/PostHog/posthog/blob/50deeae355af35b3512097ebdc592c9fadd84bb4/posthog/async_migrations/migrations/0005_person_replacing_by_version.py
  - [V] PostHog `session_replay_events` uses AggregatingMergeTree (session-level min/max timestamps, counts).
  - [U] CollapsingMergeTree (sign ±1) and VersionedCollapsingMergeTree are harder to use from application code, because you must re-emit the old row with sign -1. Most teams pick ReplacingMergeTree with argMax.
  - [U] Typical events table: `MergeTree PARTITION BY toYYYYMM(timestamp) ORDER BY (team_id, toDate(timestamp), event, cityHash64(distinct_id), uuid)` (PostHog-like), with `SAMPLE BY` the hash.
- **Async inserts** (https://clickhouse.com/docs/optimize/asynchronous-inserts):
  - [V] `async_insert=1` buffers inserts server-side and flushes on whichever comes first: `async_insert_max_data_size` (100 MiB), `async_insert_busy_timeout_ms` (200 ms, or 1000 ms on Cloud), or `async_insert_max_query_number` (450);
  - [V] the adaptive timeout has been on by default since 24.2 (minimum 50 ms);
  - [V] recommended settings are `async_insert=1, wait_for_async_insert=1`; fire-and-forget is risky;
  - [V] each flush makes at least one part per partition. Many small synchronous inserts cause "too many parts";
  - [U] the docs recommend sync batches of at least 1000 rows, ideally 10k to 100k, at about one insert per second. [U] `parts_to_throw_insert` is 3000 in recent versions.
- **Operational burden:**
  - [V] Hardware (https://clickhouse.com/docs/operations/tips): **32 GB+ RAM recommended**. Under 16 GB "can run (as low as 2 GB)" but needs tuning and ingests only at a low rate.
  - [V] Low-memory tuning:
    - mark cache 500 MB;
    - 1 query thread and `max_download_threads` 1;
    - `max_block_size` 8192;
    - parallel parsing and formatting off;
    - metric, trace and text logs off;
    - avoid S3 and Kafka engines.
  - [V] Replication needs **ClickHouse Keeper** (preferred) or ZooKeeper ≥3.4.9. Keeper is "very sensitive for disk latency", so run it on separate disks or hosts in production; co-location is fine for testing and low ingest.
  - [V] Single node needs no Keeper (plain MergeTree).
  - [U] ClickHouse is a large C++ server: the binary is several hundred MB and idles at a few hundred MB of RSS by default.
- [V] **PostHog self-host hobby:** "something equivalent to a Hetzner VM with 4 vCPU, 16GB RAM, and more than 30GB storage". It is officially unsupported, and they suggest moving to Cloud above about 300k events/month. (https://posthog.com/docs/self-host)
- [V] **Plausible CE** uses Postgres for app data plus ClickHouse for events over HTTP :8123. [U] An idle Plausible stack is about 1.8 GB vs about 400 MB for Umami, which uses Postgres or MySQL only. (https://flaviocopes.com/how-plausible-analytics-is-built/, https://dev.to/selfhostingsh/plausible-vs-umami-which-analytics-tool-3595)
- [V] Highlight.io uses ClickHouse (Cloud) for logs, traces and sessions metadata. (https://clickhouse.com/blog/overview-of-highlightio)

### B2. DuckDB (embedded)

- [V] DuckDB is MIT licensed. 2.0.0 was released 2026-10-21; earlier releases were 1.5.6 (2026-09-28), 1.5.5 (2026-07-22) and 1.5.4 (2026-06-17). (GitHub releases API)
- [V] **duckdb-rs** (https://github.com/duckdb/duckdb-rs):
  - official (duckdb org), MIT, about 985 stars;
  - versions encode the DuckDB version (`1.MAJOR_MINOR_PATCH.x`, so 1.20000.x = DuckDB 2.0.0);
  - rusqlite-like API;
  - features:
    - `bundled` compiles the C++ source via cc, the simplest option;
    - `json` and `parquet` imply bundled;
    - `appender-arrow`, `vtab`/`vtab-arrow` and `vscalar` add Arrow integration;
    - `polars` integration;
    - `httpfs`, `icu`, `tpch` and `autocomplete` need `bundled-cmake` (experimental, git checkout only), CMake, libcurl and OpenSSL; Linux and macOS only, no Windows;
  - ICU is left out of the bundled crate (crates.io 10 MB package limit), so some timezone operations need `INSTALL icu; LOAD icu` at runtime, which downloads it;
  - without `bundled`, the build can download a pinned prebuilt libduckdb or link a system one;
  - cross-compiling is "best-effort" and needs a C++ cross toolchain.
- [U] **Build cost:**
  - `bundled` adds several minutes of single-crate C++ compile on a clean build (the README only says bindgen "increased dramatically" build time, hence pregenerated bindings);
  - it adds tens of MB to the release binary (libduckdb is about 30 to 60 MB depending on extensions);
  - no official numbers were found, so measure with `cargo build --timings` and test our CI targets (musl, aarch64);
  - with CARGO_BUILD_JOBS=4 the C++ unity build is the long pole.
- **Concurrency** (https://duckdb.org/docs/2.0/connect/concurrency.html):
  - [V] **one process read-write**, or **many processes read-only**, never both;
  - [V] inside the writer process: MVCC plus optimistic concurrency, many writer threads, "appends will never conflict, even on the same table", and same-row edits conflict (retry);
  - [V] for multi-process writes, the docs point to the **Quack** remote protocol (client-server, since DuckDB 1.5.3, with a `CONNECT` statement in 2.0) or **DuckLake** with a Postgres catalog (https://duckdb.org/docs/2.0/connect/overview.html);
  - this fits "analytics service is a separate process that owns the file".
- [V] **Appender:** the docs call it the fastest way to insert from Rust, with a 10M-row transaction example. Constraint checks (NOT NULL, FK) run on `flush()`, so check its result. `appender-arrow` appends RecordBatches. (https://duckdb.org/docs/current/clients/rust/data_import)
- [V] **Memory and limits** (https://duckdb.org/docs/2.0/operations_manual/limits.html):
  - `memory_limit` defaults to **80% of RAM** and applies to the buffer manager only;
  - spill goes to the temp directory, capped by `max_temp_directory_size` (90% of free disk);
  - strings and BLOBs up to 4 GB each;
  - [U] threads default to the core count; set `threads` and `memory_limit` explicitly on a small VM (e.g. 1 to 2 GB).
- [V/U] **Performance on 100M+ rows:**
  - ClickBench (https://benchmark.clickhouse.com/) uses the `hits` dataset of about 100M rows (99,997,497), so it is the right reference;
  - DuckDB, ClickHouse and DataFusion all run most of its 43 queries in roughly 0.05 to 2 s on c6a.4xlarge (16 vCPU, 32 GB) [U exact figures; check the site];
  - on a 2 vCPU VM, expect roughly 5 to 10x slower [U].
- [V] **Parquet and S3:**
  - `read_parquet`/`COPY ... TO 'x.parquet'` are built in (the `parquet` feature in Rust);
  - S3 needs `httpfs` (cmake build, or runtime `INSTALL httpfs` from the extension repo, which needs network access and signed extensions).
- [V] **DuckLake 1.0** (2026-04-13, https://ducklake.select/2026/04/13/ducklake-10/):
  - lakehouse format: Parquet data plus a **SQL catalog in SQLite, Postgres or DuckDB**;
  - **data inlining**: commits under 10 rows (default `data_inlining_row_limit`) are stored in the catalog DB instead of a tiny Parquet file, which fixes the small-files problem for streaming ingest;
  - 1.0 also inlines updates and deletes, and adds sorted tables, bucket partitioning, VARIANT/GEOMETRY and Iceberg v3 compatibility;
  - shipped as a DuckDB extension, with the reference implementation in DuckDB 1.5.2+;
  - [U] vendor claim of up to 900x faster reads and 100x faster writes than Iceberg for streaming;
  - very relevant: "SQLite/Postgres catalog + Parquet on disk/S3" matches our small→large story, but it is DuckDB-only (no Rust-native implementation).
- [V] **pg_duckdb 1.0** (MIT; Hydra plus MotherDuck; PG 14–18) runs DuckDB's engine inside Postgres and reads Parquet, Iceberg and Delta on S3. (https://github.com/duckdb/pg_duckdb, https://motherduck.com/blog/pg-duckdb-release/)

### B3. DataFusion + Parquet on object storage (pure Rust)

- [V] **Stack:**
  - `arrow` and `parquet` (60.x) are the arrow-rs reference implementations;
  - `object_store` (0.14) is a unified async API over S3, GCS, Azure, local disk and memory. [U] It now lives in apache/arrow-rs-object-store;
  - `datafusion` (55.x) is an Apache top-level project: a vectorised, multi-threaded SQL and DataFrame engine with listing tables over partitioned Parquet (`dt=YYYY-MM-DD/` hive partitions), predicate and projection pushdown, row-group and page-index pruning, and bloom filters.
- [V] **Performance:** DataFusion 43 (Nov 2024) was the fastest single-node engine on ClickBench **for querying Parquet**, ahead of DuckDB and chDB on c6a.4xlarge (hot run, partitioned 14 GB Parquet). Gains came from StringView, a better Parquet reader, and skipping partial aggregation; performance improved more than 30% from v34 to v43. (https://datafusion.apache.org/blog/2024/11/18/datafusion-fastest-single-node-parquet-clickbench/)
- [U] **Latency characteristics:**
  - local NVMe Parquet: tens of ms to low seconds for aggregates over 100M rows;
  - S3: each query pays per-file GET latency (about 20 to 100 ms first byte) plus footer and metadata reads, so the number of files matters;
  - keep files at 64 to 512 MB, keep a metadata and footer cache, and keep a local disk cache of hot files;
  - point lookups (one person, one session) are bad on Parquet, so keep those in SQLite or Postgres.
- [U] Small-files problem: streaming ingest needs a WAL plus memtable buffer → flush to Parquet every N s or MB → background compaction (merge small files per partition) → maintain a manifest. A table format gives you that manifest.
- **Table formats:**
  - [V] **delta-rs** (`deltalake` 1.1.2):
    - Rust core with Python bindings and a DataFusion integration;
    - optimize/compaction, Z-order, vacuum, and writer protocol versions 2 to 4 and 7 (table features) are all implemented;
    - column mapping and identity columns are not;
    - merge has been "semi-done" historically [U current];
    - log-based (JSON commit files plus checkpoints), so concurrent S3 writers need a locking provider (DynamoDB) or S3 conditional writes [U].
    - Source: https://github.com/delta-io/delta-rs
  - [V] **iceberg-rust** (`iceberg` 0.11.0, Nov 2026):
    - catalogs (REST, Glue, SQL, memory) and an `iceberg-datafusion` integration;
    - 0.8 (Jan 2026) added delete-file handling in DataFusion and "Advanced Writers";
    - **compaction lives mostly outside core**, in nimtable/iceberg-compaction (DataFusion-based: binpack, sort, z-order roadmap) and a DataFusion Comet PR;
    - [U] the write path is less mature than Java Iceberg's;
    - needs a catalog service (REST, or a SQL catalog in SQLite or Postgres).
    - Sources: https://iceberg.apache.org/blog/apache-iceberg-rust-0.8.0-release/, https://github.com/nimtable/iceberg-compaction
  - [V] DuckLake (see B2) is the simplest format conceptually, but has no Rust implementation.
  - [U] Lance (`lance` 14.x) and Vortex are newer columnar formats with fast random access. Interesting, but not standard.
- **Products built this way (proof the architecture works):**
  - [V] **InfluxDB 3 Core**: the FDAP stack (Flight, DataFusion, Arrow, Parquet); MIT/Apache-2.0; GA April 2025; local disk or "diskless" S3; writes buffered in memory and snapshotted to Parquet; last-value and distinct-value caches; embedded Python plugins; the early 72 h query limit was lifted. (https://github.com/influxdata/influxdb, https://influxdata.com/blog/influxdb3-open-source-public-alpha)
  - [V] **GreptimeDB**: Rust, Apache-2.0; Arrow in memory, Parquet on disk; OpenDAL for object storage; DataFusion queries; `mito` engine; **standalone single binary** or distributed (frontend, datanode, metasrv on etcd). (https://github.com/GreptimeTeam/greptimedb)
  - [V] **OpenObserve**: Rust, AGPL-3.0 (plus an enterprise license); Parquet on S3, GCS, Azure or MinIO; DataFusion; stateless nodes (NATS for coordination). [U] Memtable 256 MB / WAL 128 MB flush, upload every 10 s, merge files to 256 MB. (https://openobserve.ai/platform.md, https://knowledge-base-4bl.pages.dev/observability/openobserve/)
  - [V] **Parseable**: Rust, AGPL-3.0. Ingest goes to Arrow in memory, is staged on local disk, then converted to Parquet and uploaded to S3; a manifest tracks files by column and time range; DataFusion with Postgres-compatible SQL; local SSD cache. (https://www.tigrisdata.com/blog/case-study-parseable/)
  - [V] **Quickwit**: Rust on Tantivy; search indexes ("splits") stored on object storage. **Datadog acquired it in Jan 2025** and committed to relicense to Apache-2.0. [U] Whether the relicense shipped, and its development status since, is unconfirmed. Tantivy 0.27 is still released. (https://quickwit.io/blog/quickwit-joins-datadog, https://www.datadoghq.com/blog/datadog-acquires-quickwit/)
  - [V] **Datadog Husky** (their own event store): an "unbundled, distributed, schemaless, vectorized column store" on S3/GCS. Writers read Kafka, buffer briefly, and write custom columnar "fragments" (Parquet-like, one row group with many pages); metadata lives in FoundationDB with exactly-once event IDs; readers cache; compaction merges fragments. (https://www.datadoghq.com/blog/engineering/introducing-husky/, https://www.datadoghq.com/blog/engineering/husky-storage-compaction/, https://www.infoq.com/news/2023/06/datadog-husky-data-ingestion)
  - [U] **Axiom** is proprietary (Go); its own columnar format on object storage with serverless query.
- [U] **Binary and build:** DataFusion is pure Rust, so cross-compiling is easy (musl, aarch64). But it is large: the full feature set adds about 20 to 40 MB to the release binary and a heavy compile (hundreds of crates). Trim with `default-features = false` plus `parquet`; use `datafusion-sql` only if you need it.

### B4. Postgres analytics extensions

- [V] **TimescaleDB** (Tiger Data):
  - hypertables (time-chunked), continuous aggregates, and hypercore (row store for recent chunks, columnstore policy converting older chunks);
  - compression is typically 90 to 98% (vendor figure); an event-log anecdote went from 220 GB to 25 GB;
  - **licensing:** the Apache-2 edition lacks compression/columnstore functions (`add_compression_policy`, `compress_chunk`...). Those are in the Community edition under the **Tiger Data License (TSL)**, which is free to self-host but can't be sold as a DBaaS;
  - `SHOW timescaledb.license;`;
  - many managed Postgres providers ship only the Apache edition.
  - Sources: https://www.tigerdata.com/docs/about/latest/timescaledb-editions, https://www.tigerdata.com/docs/build/how-to/basic-compression, https://dev.to/polliog/timescaledb-compression-from-150gb-to-15gb-90-reduction-real-production-data-bnj
- [V] **pg_duckdb 1.0**: MIT, PG 14–18; DuckDB execution inside Postgres; reads Parquet, CSV, Iceberg and Delta on S3; MotherDuck integration. It is good for querying archived Parquet from Postgres, but the native PG heap stays row-oriented.
- [V] **Citus columnar**: AGPL-3.0, PG 14–18, actively packaged, the most established. AGPL is a concern for embedding or distribution. (https://pigsty.io/ext/e/citus_columnar/)
- [V/U] **Hydra columnar**: AGPL-3.0. Pigsty marks it "obsolete, no longer maintained" with PG 14–16 only [U upstream confirmation]. Hydra's effort moved into pg_duckdb.
- [V] **ParadeDB pg_analytics**: deprecated and the repo was archived on 2025-03-19. ParadeDB now focuses on `pg_search` (Tantivy-based BM25). (https://github.com/paradedb/pg_analytics, https://docs.paradedb.com/changelog/0.15.9)
- [V] **pg_mooncake** (MIT, columnstore mirrored to Iceberg/Delta): **Databricks acquired Mooncake Labs on 2025-10-01** and folded it into Lakebase. [U] The independent OSS future is uncertain. (https://www.databricks.com/blog/mooncake-labs-joins-databricks-accelerate-vision-lakebase)
- Takeaway: on the Postgres path, plain PG with BRIN on time, monthly declarative partitions, JSONB properties plus generated columns for hot properties, and rollup tables gets far. Offer TimescaleDB as optional (TSL is fine for self-hosters). None of these is pure Rust or embeddable.

### B5. Ingest benchmarks (indicative only; hardware differs)

- **SQLite:**
  - [V] benbjohnson/sqlite-bench, WAL, 1M rows in 1000-row batches of 100 B rows: about **317k inserts/s** with synchronous default (FULL), about 393k with NORMAL, about 478k with OFF (unsafe). (https://github.com/benbjohnson/sqlite-bench)
  - [V] Rust port, WAL, 10k-row batches: about **519k inserts/s**. (https://github.com/psarna/sqlite-bench-rs)
  - [V] Anders Murphy, M1 Pro, 1B-row table, 1 writer plus 8 readers, WAL, NORMAL. These are update transactions, not appends: 44k TPS unbatched; **186k TPS with a dynamic batcher** (162k with FULL); about 100k to 122k TPS with savepoint-per-tx plus concurrent reads. Networked Postgres on the same workload gave 13.8k TPS with no network latency and 702 TPS at 10 ms latency. (https://andersmurphy.com/2025/12/02/100000-tps-over-a-billion-rows-the-unreasonable-effectiveness-of-sqlite.html)
  - [U] Rule of thumb: autocommit gives about 50 to 1000 inserts/s (fsync-bound); 1k to 10k-row transactions give 50k to 500k rows/s.
  - Realistic events with JSON properties (about 0.5 to 2 KB) plus 2 or 3 indexes: expect roughly **20k to 100k events/s on a small VM** [U estimate]. Single writer: funnel all writes through one task.
- **Postgres:**
  - [V] TigerData: 1M rows, COPY 4.3 s (about **232k rows/s**) vs batched INSERT with 20k-row batches at 32.5 s (about **31k rows/s**). (https://www.tigerdata.com/learn/testing-postgres-ingest-insert-vs-batch-insert-vs-copy)
  - [V] PG 18.4 on m7i.2xlarge: per-row INSERT 983/s; multi-row INSERT 32.9k/s; COPY 255.8k/s; unlogged COPY 308.6k/s. (https://karan.is-a.dev/blog/postgres-copy-ingest/1m-rows-into-postgres-in-1-29-seconds)
  - [V] Java/JDBC: batch 162k/s, reWriteBatchedInserts 292k/s, COPY BINARY about **712k rows/s**. (https://hn.jakechampion.name/reader/https://sqg.dev/blog/java-postgres-insert-benchmark/)
  - So COPY is about 4 to 10x faster than multi-row INSERT. From Rust, use `tokio-postgres` `copy_in` with binary COPY (`BinaryCopyInWriter`), or `sqlx` `copy_in_raw`. [U] An alternative is `INSERT ... SELECT * FROM unnest($1::text[], $2::timestamptz[], ...)`, which comes close to COPY with one round trip.
- **ClickHouse:**
  - [V] Tinybird single node: 10M rows in 4.6 s (about **2.1M rows/s**), and about 8.8M rows/s tuned, with rows of about 126 B. Hardware not stated. (https://www.tinybird.co/blog/md/1b-rows-per-second-clickhouse)
  - [V] Docs: "millions of row inserts per second"; batch at least 1000 rows.
  - [U] On a 2 vCPU / 4 GB VM, expect a few hundred k rows/s with large batches. Memory is the constraint more than CPU.
- **DuckDB:**
  - [U] No authoritative Rust appender rows/s figure was found.
  - [U] Community reports put the row-by-row Appender at about 1 to 5M narrow rows/s per thread and Arrow appends faster. Wide JSON rows are slower: the JSON type is stored as a string, and parsing on read dominates.
  - Benchmark this ourselves.
- [U] **Parquet writes (arrow-rs):** encoding plus zstd at hundreds of MB/s per core, i.e. millions of events/s from buffered RecordBatches. The bottleneck is batching and compaction, not encoding.
- Context: 1M events/day averages about 12 events/s (peaks of maybe 100 to 500/s). 100M events/month is about 40/s average. **Every option comfortably handles small-install ingest. Query speed over large history and storage cost are what differ.**

### B6. Recommendation criteria and pick

Criteria for leptoskit, ranked:

1. A single static Rust binary that runs on one small VM (1 to 2 vCPU, 1 to 4 GB RAM), with no extra server.
2. A pure-Rust build. The repo already has large builds (CARGO_BUILD_JOBS=4), and cross-compiling to aarch64 and musl must keep working.
3. A permissive license (MIT/Apache) so it can be embedded and redistributed.
4. Cheap long-term retention (object storage at about $0.015 to 0.023/GB-month, or local disk).
5. Interactive queries (trends, funnels, retention) at 10M to 1B events.
6. Upgrade path to a dedicated cluster without changing the app API.

How the options score:

| Option                                           | Embedded, no extra server                | Pure Rust                                                  | License | Cost                    | Query at 100M+                           | Ops on a small VM                               |
| ------------------------------------------------ | ---------------------------------------- | ---------------------------------------------------------- | ------- | ----------------------- | ---------------------------------------- | ----------------------------------------------- |
| SQLite (rusqlite / sqlx)                         | yes                                      | C, but tiny and easy                                       | PD      | disk                    | slow (row store, JSON)                   | trivial                                         |
| Postgres                                         | no (server)                              | n/a                                                        | PG      | disk                    | moderate; extensions help                | moderate                                        |
| ClickHouse                                       | **no**; 32 GB recommended, Keeper for HA | n/a                                                        | Apache  | disk/S3                 | best                                     | **heavy**                                       |
| DuckDB (duckdb-rs bundled)                       | yes                                      | **no** (C++ build, cross-compile pain, binary +tens of MB) | MIT     | disk/Parquet/S3         | excellent                                | easy; one process RW                            |
| DataFusion + Parquet (+ object_store)            | yes                                      | **yes**                                                    | Apache  | **cheapest** (S3/local) | excellent on Parquet; poor point lookups | easy, but we build WAL, compaction and manifest |
| Delta (delta-rs) / Iceberg (iceberg-rust) on top | yes                                      | yes                                                        | Apache  | cheap                   | same as above                            | adds catalog and compaction maturity risk       |
| DuckLake                                         | via DuckDB only                          | no                                                         | MIT     | cheap                   | excellent                                | SQLite/PG catalog fits us, but C++              |

Recommended layered design:

- **Tier 0 (default, small installs):**
  - SQLite (WAL, NORMAL, one writer task batching 1k to 10k rows or about 100 to 250 ms) or Postgres as **both the metadata store and the hot event store**;
  - metadata: persons, distinct_id map, sessions, definitions, cohorts, dashboards;
  - events go in an append-only table with typed core columns (team_id, ts, event, distinct_id, person_id, session_id, uuid) plus JSON properties;
  - rollup tables are maintained at ingest.
  - This covers about 10 to 50M events comfortably [U].
- **Tier 1 (scale-up, same binary, feature-flagged):**
  - **Parquet segments + DataFusion**, pure Rust, Apache-2.0;
  - flush buffered events into Arrow RecordBatches → Parquet (zstd, sorted by team_id, event, ts) under `events/team_id=X/dt=YYYY-MM-DD/part-<ulid>.parquet` on local disk or S3 via `object_store`;
  - keep the **manifest in the existing SQLite or Postgres DB** (DuckLake-style: file list, min/max ts, row counts, snapshots). This avoids depending on immature Rust Iceberg/Delta writers. Inline small recent data in the DB ("hot tail" = the Tier-0 table), as DuckLake does;
  - background compaction merges small files per partition up to about 128 to 512 MB;
  - queries union the hot SQL tail with the Parquet history through a DataFusion `TableProvider`;
  - persons and sessions stay in SQL (point lookups and upserts).
  - Optionally adopt delta-rs later if external engines (Spark, DuckDB, Trino) must read the data. Delta is more mature in Rust than Iceberg today.
- **Tier 2 (large installs): an optional ClickHouse sink** via the official `clickhouse` crate (RowBinary + inserter, or server async_insert), using PostHog-style schemas:
  - `events` MergeTree;
  - `person` and `person_distinct_id` as ReplacingMergeTree(version) with is_deleted;
  - `sessions` as AggregatingMergeTree.
  - It is for operators who already run CH, behind the same query trait.
- **Why not DuckDB as the default:**
  - it is the best query engine per unit of effort, but the bundled C++ build adds minutes and tens of MB, complicates cross-compiles and Windows `httpfs`, has a single-process RW lock, and needs runtime extension downloads for S3 and ICU;
  - keep it optional (`--features duckdb`), or recommend DuckDB/DuckLake as an **external reader** of our Parquet files.
- **Why not ClickHouse as the default:** a separate heavy server, 32 GB recommended, and Keeper for HA. That conflicts with "easy on one small VM" (PostHog's own hobby stack needs 4 vCPU / 16 GB).
- **Postgres extensions:** document TimescaleDB (TSL) and pg_duckdb as operator options. Don't depend on them. Avoid AGPL Citus/Hydra and the archived pg_analytics.

---

## PART C: rrweb session replay storage in open-source tools

- [V] **PostHog** (https://posthog.com/handbook/engineering/session-replay/session-replay-architecture, https://posthog.com/docs/how-posthog-works/recordings-ingestion):
  - pipeline: capture → Kafka → blob-ingestion consumer;
  - buffers per session and flushes after at most 10 s buffer age or 100 MB;
  - writes **batch files to S3** (MinIO in self-host) that contain multiple **per-session blocks, each Snappy-compressed**;
  - the content is NDJSON lines of `[windowId, event]`;
  - metadata goes to the ClickHouse `session_replay_events` table (AggregatingMergeTree: session_id, team_id, distinct_id, min_first_timestamp, max_last_timestamp, counts);
  - playback: the API returns a block list with **byte ranges**, plus presigned URLs (about 60 s) [V via a handbook mirror]; the client fetches ranges directly from S3 and decompresses Snappy in the browser;
  - storing recordings in ClickHouse is deprecated as "impractical and expensive at scale";
  - cloud retention is 1 month (free) or 3 months (paid).
- [V] **OpenReplay** (https://github.com/openreplay/openreplay, backend/pkg/storage/storage.go):
  - the tracker uses its own binary message format, not rrweb;
  - pipeline: ingest → Redis streams → sink (writes per-session files to local disk) → ender (detects session end) → storage service;
  - uploads `<sid>/dom.mobs` (start segment), `<sid>/dom.mobe` (end segment) and `<sid>/devtools.mob`;
  - files are **zstd-compressed** with `klauspost/compress/zstd` at SpeedFastest, streamed to S3-compatible storage (MinIO by default), with **optional per-project encryption**;
  - assets (CSS, images) are cached in a separate bucket that replays need.
  - Docs: https://docs.openreplay.com/configuration/recordings-storage
- [V] **Highlight.io** (https://github.com/highlight/highlight, backend/storage/storage.go; repo still active 2026-10):
  - raw events are buffered in **Redis sorted sets** during the session (`PushRawEvents` with `redis.Z` scores = timestamps);
  - then written as **Brotli-compressed** files (`Content-Encoding: br`) to S3 or the local filesystem;
  - key layout: `<project>/<session>/<payloadType>`, with payload types `session-contents-compressed` (chunked as `-0000`, `-0001`...), `network-resources-compressed`, `web-socket-events-compressed` and `timeline-indicator-events`;
  - direct download URLs let the browser decompress natively thanks to `Content-Encoding: br`;
  - ClickHouse holds session metadata for search.
  - [U] It was acquired by LaunchDarkly in 2025.
- [V] **Datadog:** rrweb-based, compressed in a web worker, under 100 kB/min.
- Common pattern to copy:
  - (1) the client compresses (fflate/deflate in a worker) and sends segments every few seconds;
  - (2) the server appends segments per session to a local file or buffer;
  - (3) on idle or size threshold, flush a compressed blob (zstd for server-side efficiency, or gzip/brotli so browsers can decode via `Content-Encoding`) to disk or S3 at `replays/<team>/<yyyy-mm-dd>/<session_id>/<seq>.bin`;
  - (4) SQL holds session metadata (start/end, duration, counts, frustration counts, URL list) plus the segment index (seq, byte offset/length, ts range);
  - (5) serve by byte range or presigned URL;
  - (6) set retention with object lifecycle rules per `dt` prefix.
  - Never store replay blobs in the analytics column store.
- [U] Typical replay size is about 0.1 to 1 MB compressed per minute of active session (Datadog <100 kB/min is a target).

---

## Open questions / to verify next

- Datadog Live Tail exact UI limits (rate cap, buffer size, pause semantics). These are not on the current docs page.
- duckdb-rs `bundled` compile time and binary size on our CI targets. Measure.
- DuckDB appender and DataFusion→Parquet write throughput for our event schema on a 2 vCPU VM. Measure.
- iceberg-rust write and compaction maturity at 0.11, and delta-rs concurrent writers on S3 via conditional PUT.
- The Quickwit relicense outcome and its maintenance after the Datadog acquisition.
