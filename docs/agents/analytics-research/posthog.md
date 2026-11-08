# PostHog research for a Rust, self-hosted, PostHog-class analytics service

Researched 2026-11-07. Sources are PostHog docs (posthog.com), the PostHog monorepo (`github.com/PostHog/posthog`, master as of this date), `PostHog/posthog-js`, `PostHog/posthog-python`, Hacker News, and GitHub issues.

How claims are marked:

- **[code]**: read directly in source code (file path given).
- **[docs]**: from a posthog.com page (URL given).
- **[unverified]**: from prior knowledge or secondary sources, not confirmed today.

Repo paths are relative to `PostHog/posthog` unless noted otherwise.

---

## 0. TL;DR for the design doc

- **What PostHog is now.** One event stream (ClickHouse `events`) plus a persons store (Postgres, with a ClickHouse mirror). Around 20 products sit on top of it, all queried through a single SQL dialect (HogQL) and a typed "Query" node schema.
- **Hot path.**
  - Rust `capture` (axum) decompresses, normalizes, and fixes clock skew.
  - It writes to Kafka `events_plugin_ingestion`, keyed by `token:distinct_id`.
  - Node ingestion then does transformations, person processing in Postgres, and group/person enrichment.
  - It produces to `clickhouse_events_json`, which a ClickHouse Kafka engine and materialized view read into the sharded `events` (ReplacingMergeTree).
  - The ingestion side is moving to Rust (`ingestion-consumer`, `personhog-*`, `kafka-deduplicator`, `property-defs-rs`, `cymbal`).
- **Feature flags.** A Rust service handles `POST /flags?v=2`. Bucketing is deterministic: `sha1("{flag_key}.{bucketing_value}{salt}")`, take the first 15 hex digits, divide by `0xFFFFFFFFFFFFFFF`. Server SDKs poll the flag definitions and evaluate locally.
- **Persons are the expensive, fragile part.** PostHog itself made person profiles opt-in (`person_profiles: 'identified_only'`), bills identified events at up to 4x anonymous ones, and caps sustained per-distinct_id throughput by rerouting to overflow without person processing.
- **Self-hosting.** PostHog actively discourages it. The hobby compose is MIT, "unsupported", rated at roughly 100k events/month, pulls `:master`, and pulls in a growing list of dependencies (ClickHouse, Kafka/Redpanda, ZooKeeper, Postgres, Redis/Valkey, object storage, Temporal, Elasticsearch/OpenSearch, and more). This is the main opening for a Rust single-binary competitor.

---

## 1. Product surface (2025–2026)

The products index lists these top-level products [docs: https://posthog.com/llms.txt]: Product analytics, Web analytics, Session replay, Feature flags, Experiments, Error tracking, Surveys, AI observability (formerly "LLM analytics/observability"), Logs, Data warehouse, CDP / Data pipelines, Endpoints, Workflows, PostHog AI (formerly "Max"), Support/Conversations, Customer analytics, Replay Vision.

The docs tree also has: revenue-analytics, mcp-analytics, metrics (application metrics), distributed-tracing, semantic-layer, business-knowledge, ai-evals, tasks (cloud coding agents), self-driving, health-checks (ingestion warnings, sync failures), toolbar, sql, vscode-extension, PostHog Desktop, Slack app, CLI, MCP server [docs: llms.txt].

### Product analytics: insight types

Code enums are in `posthog/schema_enums.py` [code].

- **Trends** (`TrendsQuery`).
  - Series are events, actions, or data-warehouse tables.
  - Math:
    - `BaseMathType`: total, dau, weekly_active, monthly_active, unique_session, first_time_for_user, first_matching_event_for_user.
    - `PropertyMathType`: avg, sum, min, max, median, p75, p90, p95, p99.
    - Count-per-actor: avg/min/max/median/p75/p90/p95/p99 of count per actor.
    - Group-unique (unique groups of type N), plus HogQL expression math.
  - Formulas across series (A/B, etc.), breakdowns (multi-breakdown), compare-to-previous-period.
  - Display types: Auto, line, bar, stacked/unstacked bar, area, cumulative line, BoldNumber, Metric, pie, donut, proportion bar, bar value, table, WorldMap, CalendarHeatmap, TwoDimensionalHeatmap, BoxPlot, SlopeGraph, ScatterPlot.
  - Intervals: second, minute, hour, day, week, month, quarter, year.
  - Breakdown limits: `BREAKDOWN_VALUES_LIMIT = 25` (300 for countries); values are capped at 400 chars [code: `posthog/hogql/constants.py`].
- **Funnels** (`FunnelsQuery`). Viz types are steps, time_to_convert, trends, and flow. Section 10 has the semantics.
- **Retention** (`RetentionQuery`). Types are retention_recurring, retention_first_time, and retention_first_ever_occurrence. See section 10.
- **Paths** (`PathsQuery`, plus a `PathsV2Query`). Path types are `$pageview`, `$screen`, custom_event, and hogql.
- **Stickiness** (`StickinessQuery`).
  - How many actors performed the event on exactly N intervals (days) within the range.
  - Computation modes: non_cumulative, and cumulative ("N or more") [code]. The definition is [unverified]; the docs page didn't define it.
- **Lifecycle** (`LifecycleQuery`).
  - Statuses: new (person created in the interval and active), returning (active previous and current), resurrecting (inactive previous, active now), dormant (active previous, not now; plotted negative).
  - Requires identified persons [docs: https://posthog.com/docs/product-analytics/lifecycle].
- **SQL insights** (`HogQLQuery` / `DataVisualizationNode`), **Calendar heatmap** (`CalendarHeatmapQuery`), and an events/data table (`EventsQuery`, `ActorsQuery`, `SessionsQuery`, `GroupsQuery`) [code: NodeKind enum].

### Other products and features

- **Web analytics.** Section 11.
- **Session replay.** Section 7. Replay Vision is AI "scanners" over replays with alerts [docs: llms.txt].
- **Feature flags.** Section 5.
  - **Early access features**: users opt in; stored as person property `$feature_enrollment/<flag_key>` [code: posthog-python `feature_flags.py`].
  - **Remote config flags.**
  - **Flag dependencies** (`flag_evaluates_to` operator).
  - **Evaluation contexts / tags.**
  - **Device-ID bucketing.**
- **Experiments.** Section 6. Product-flag experiments, no-code web experiments (`disable_web_experiments: true` by default, still beta [code: posthog-js `posthog-core.ts`]), holdouts, shared metrics, and experiment "health checks" (SRM, bucketing changed, forced variant, zero exposures, and others) [code: `products/experiments/backend/health/checks/`].
- **Surveys.**
  - Popover, API, and hosted surveys.
  - Events `survey shown`, `survey sent`, `survey dismissed` with `$survey_id`, `$survey_response_<question_id>`, `$survey_questions` [docs: https://posthog.com/docs/api/capture].
  - Billed per response. The free tier is widely cited as 1,500 responses/month [unverified].
- **Heatmaps.** Click, rageclick, mouse-move, and scroll-depth heatmaps.
  - posthog-js `capture_heatmaps` / `enable_heatmaps`.
  - Separate Kafka topic `heatmaps_ingestion` [code: `rust/capture/src/config.rs`].
  - Heatmap queries are capped at 1M points (`MAX_SELECT_HEATMAPS_LIMIT`) [code].
- **Autocapture extras.** Dead clicks (`capture_dead_clicks`, `$dead_click`) and rageclicks (`rageclick`), both config [code: posthog-js types].
- **Error tracking.**
  - `$exception` events, issues and fingerprints, source maps / symbol sets.
  - Uses the Rust `cymbal` service (fingerprinting, symbolication, issue linking, Temporal workflows for issue-created/reopened/spiking notifications) [code: `rust/cymbal/README.md`].
  - Kafka topic `ingestion-errortracking-main` [code].
- **Data warehouse.**
  - Sources such as Stripe, HubSpot, Postgres, and many more. Views and materialized "saved queries". Joins to persons/groups.
  - As of mid-2026 it was rebuilt on per-organization single-tenant DuckDB instances. They use a DuckLake catalog on S3, sleep when idle, and expose a Postgres-wire endpoint, with a "DuckHog" extension.
  - ClickHouse remains the engine for product analytics [docs: https://posthog.com/blog/why-we-rebuilt-our-data-warehouse]. Service `duckgres` appears in the compose file [code: `docker-compose.base.yml`].
- **CDP / Data pipelines** [docs: https://posthog.com/docs/cdp].
  - Sources (incoming webhooks plus warehouse sources).
  - Transformations that run at ingestion before storage (e.g., GeoIP, property filter).
  - Realtime destinations (webhook, Slack, HubSpot, and others), built as "Hog functions" from templates.
  - Batch exports to warehouses on a schedule. Targets include S3, Snowflake, BigQuery, Postgres, Redshift, Databricks, and Azure Blob [unverified].
- **Hog language** [docs: https://posthog.com/docs/hog].
  - Syntax-compatible with HogQL expressions: `:=` assigns and `=` is equality. 1-indexed arrays, f-strings, `??`, `?.`, lambdas, regex operators.
  - Compiled to bytecode for a stack-based HogVM; TypeScript and Python VMs behave identically.
  - The VM can serialize its state before `fetch()` and resume elsewhere after an async fetch service handles the call via a queue.
- **AI observability (LLM analytics).**
  - Events `$ai_generation`, `$ai_trace`, `$ai_span`, `$ai_embedding`. Properties include `$ai_trace_id`, `$ai_span_id`, `$ai_parent_id`, `$ai_model`, `$ai_provider`, `$ai_input`, `$ai_output_choices`, `$ai_input_tokens`, `$ai_output_tokens`, `$ai_latency`, `$ai_time_to_first_token`, `$ai_is_error`, `$ai_error`, `$ai_http_status`, `$ai_total_cost_usd`, `$ai_input_cost_usd`, `$ai_output_cost_usd` (cost computed automatically if omitted) [docs: https://posthog.com/docs/ai-observability/manual-capture]. `$ai_metric` and `$ai_feedback` also exist [unverified].
  - A dedicated capture lane: `/i/v0/ai`, `/i/v0/ai/batch`, `/i/v0/ai/otel` (OpenTelemetry ingest). Topic `events_plugin_ingestion_ai`, with a 25MB sum-of-parts limit [code: `rust/capture/src/router.rs`, `config.rs`].
  - Heavy properties like `$ai_input` are stripped from the shared events output and kept in `clickhouse_ai_events_json` [code: `rust/property-defs-rs/README.md`].
  - AI evals with datasets.
- **Revenue analytics.** Revenue events plus Stripe sources. HogQL tables `persons_revenue_analytics` and `groups_revenue_analytics`, and an exchange-rate table [code: `posthog/hogql/database/schema/`].
- **Logs, metrics, distributed tracing.** OTel-based; services `capture-logs` and `logs-ingestion`; topics `ingestion-traces` / `ingestion-metrics` [code].
- **Marketing analytics.** Ad-cost imports, attribution, conversion goals; many `MARKETING_ANALYTICS_*` query kinds [code].
- **Toolbar.** An in-page overlay that injects JS via `loadToolbar`. It is used to create actions by element selection, view heatmaps, override flags, and inspect events.
- **Actions.** Saved, named definitions over events: event name plus URL/selector/text/href matchers on `elements_chain`, or SQL. Usable anywhere an event is.
- **Cohorts.** Dynamic (recomputed) and static (CSV/lists, table `person_static_cohort`) [code: `posthog/models/person/sql.py`]. Newer "behavioral cohorts" use realtime stream processing [code: `rust/cohort-stream-processor`, `cohort-core`].
- **Collaboration.** Annotations (time markers on charts), dashboards (tiles with filters and variables), notebooks (rich docs embedding insights/replays/SQL), alerts (threshold alerts on insights and SQL; `LimitContext.SQL_ALERT` [code]), subscriptions (scheduled email/Slack delivery of insights/dashboards).
- **Group analytics.** Up to 5 group types per project, with event columns `$group_0..$group_4` and `group0..4_properties` [code: `posthog/models/event/sql.py`]. Created and updated via `$groupidentify`.
- **Data management.**
  - Event definitions, property definitions, event-property pairs, verified/hidden events, property types. Written by Rust `property-defs-rs` into `posthog_eventdefinition`, `posthog_propertydefinition`, `posthog_eventproperty` [code].
  - Ingestion warnings UI (health-checks).
  - Historical-migration imports via a `batch-import-worker` [code].
- **Additional products not in the brief.**
  - Workflows: messaging and journeys with email/SMS/push channels [docs: llms.txt].
  - Endpoints: customer-facing, cached SQL endpoints.
  - Product tours (`disable_product_tours` [code]).
  - Conversations/support widget.
  - Customer analytics (accounts/usage).
  - Semantic layer.
  - MCP analytics.
  - PostHog AI agent plus MCP server.
  - Cookieless mode (`cookieless_mode: 'always' | 'on_reject'`; server-side hashing; partition key `token:ip` [code]).
  - Livestream (`livestream` service).
  - Cloud tasks, PostHog Code.

### Pricing shape (relevant to "complaints")

- Each product is metered separately.
- Free monthly tiers widely cited: 1M events, 5k web recordings, 1M flag requests, 1,500 survey responses, 100k exceptions [unverified; the pricing page didn't render].
- Paid rates: feature flags from about $0.0001/request, replay from about $0.005/recording, events from about $0.00005 [unverified, secondary: https://dev.to/beton/posthog-pricing-teardown-2026-57oo, https://www.withorb.com/pricing-atlas/posthog].
- Identified events ("person profiles") cost up to 4x anonymous events [docs: https://posthog.com/docs/data/anonymous-vs-identified-events].
- Experiments bill via flag requests.
- Local-evaluation polls are billed as flag requests:
  - A poll that returns changed definitions counts as 10 requests.
  - The docs contradict themselves on unchanged polls: one passage says 1 request, another says ETag `304` polls are free [docs: https://posthog.com/docs/feature-flags/local-evaluation].

---

## 2. Event model

### Wire shape

The `RawEvent` struct in capture [code: `rust/common/types/src/event.rs`]:

```
token        (aliases "$token", "api_key")
distinct_id  (alias "$distinct_id"; arbitrary JSON value accepted, stringified)
uuid         (optional; "" -> None; generated server-side if absent, UUIDv7)
event        (string)
properties   (map)
timestamp    (optional string, any parseable format)
offset       (optional i64 ms: "event happened N ms before now")
$set, $set_once (optional maps at top level, also accepted inside properties)
```

UUIDv7 generation [code: `rust/capture/src/sinks/sink.rs` uses `now_v7`; exact path unverified].

Capture wraps this in `CapturedEvent { uuid, distinct_id, ip, data (re-serialized RawEvent JSON string), now, sent_at, token, event, timestamp (computed), is_cookieless_mode, historical_migration }` [code]. Kafka headers carry `token`, `distinct_id`, `session_id`, `timestamp`, `event`, `uuid`, `force_disable_person_processing`, `historical_migration`, `skip_heatmap_processing`, and DLQ reason/step [code].

Distinct ID rules:

- Max 200 chars; longer values are truncated and an ingestion warning is raised.
- Events with no name or an empty distinct_id are dropped but still get `200` [docs: https://posthog.com/docs/api/capture].
- `distinct_id` is read from the top level first, then `properties.distinct_id`.
- `$group_type` / `$group_key` are limited to 400 chars.

### ClickHouse row

Table `events`, sharded [code: `posthog/models/event/sql.py`]:

- Columns: `uuid UUID`, `event`, `properties String ZSTD(3)`, `timestamp DateTime64(6,'UTC')`, `team_id Int64`, `distinct_id`, `elements_chain` (autocapture DOM chain), `created_at`, `person_id UUID`, `person_created_at`, `person_properties ZSTD(3)`, `group0..4_properties`, `group0..4_created_at`, `person_mode Enum8('full','propertyless','force_upgrade')`, `historical_migration Bool`, `inserted_at`, `_timestamp`, `_offset`, `consumer_breadcrumbs`.
- Materialized columns: `$group_0..4`, `$window_id`, `$session_id`, `$session_id_uuid UInt128`, `elements_chain_href`/`_texts`/`_ids`/`_elements`. Minmax skip indexes on these.
- 10 per-team "dynamic materialized" slots `dmat_string_0..9`, mapped via a dictionary of (team_id, slot) -> property.
- A newer `events_json` table uses native ClickHouse JSON with typed subcolumns and a cap on max dynamic paths.

### Reserved / `$` events and properties

- **`$pageview`**: carries `$current_url`, `$pathname`, `$host`, `$referrer`, `$referring_domain`, and `$session_id`. JS default `capture_pageview: true`; `'history_change'` for configs with `defaults >= '2025-05-24'` (SPA support) [code: posthog-js `posthog-core.ts`].
- **`$pageleave`**: default `'if_capture_pageview'`. Sent via `sendBeacon` on unload; carries scroll depth (`$prev_pageview_*` props [unverified exact names]).
- **`$screen`**: mobile; `$screen_name`.
- **`$autocapture`**: clicks, changes, submits. `$event_type`, plus `elements_chain` (a serialized ancestor chain like `a.cls:attr_id="x"href="..."nth-child="1"text="..."`).
- **`$rageclick`, `$dead_click`, `$copy_autocapture`** (`capture_copied_text`) [code: posthog-js types].
- **`$identify`**: properties `$anon_distinct_id`; `$set`/`$set_once`. A merge trigger (section 2.2).
- **`$create_alias`**: `distinct_id` plus `properties.alias`; the alias is merged into `distinct_id`.
- **`$merge_dangerously`**: same as alias but without safeguards.
- **`$set` / `$set_once` / `$unset`**: person property ops on any event. They are NOT stored as queryable event properties; only person state changes [docs: https://posthog.com/docs/how-posthog-works/ingestion-pipeline].
- **`$groupidentify`**: `$group_type`, `$group_key`, `$group_set`. Events link to groups via `properties.$groups = {"company": "id"}`, which does NOT create the group [docs: capture API].
- **`$exception`**: `$exception_list[]` with `{type, value, mechanism{handled, synthetic}, stacktrace{frames}}`, plus `$exception_fingerprint`, `$exception_level`, `$issue_name`, `$issue_description` (255-char cap), and `$exception_steps` breadcrumbs [docs: https://posthog.com/docs/error-tracking/capture].
- **`$ai_*`**: section 1.
- **`$feature_flag_called`**: `$feature_flag`, `$feature_flag_response`, `$feature_flag_id`, `$feature_flag_version`, `$feature_flag_reason`, `$feature_flag_request_id`, `$feature_flag_evaluated_at` [code: posthog-js `packages/node/src/feature-flag-evaluations.ts`].
  - Events also carry `$feature/<flag_key>` = value [code: `FEATURE_FLAG_PROPERTY_PREFIX` in `posthog/hogql/constants.py`] and `$active_feature_flags`.
- **`$snapshot`**: replay. Goes to `/s/` with `$snapshot_items`, `$session_id`, `$window_id`, `$snapshot_source`.
- **`$web_vitals`**: LCP/CLS/INP/FCP [unverified list; posthog-js has `web_vitals_allowed_metrics`].
- **`$heatmap`** data and **`$$client_ingestion_warning`** (client-side rate limiter tripped) [code].
- **`$session_id`**: UUIDv7. Rotates after 30 min idle (configurable 60s–10h) or 24h max length [code: posthog-js `sessionid.ts`].
- **`$window_id`**: per-tab ID.
- **`$device_id`**: persistent per-browser ID (`get_device_id` hook). Reset only by `reset(true)` [docs: identify-users].
- **`$process_person_profile: false`**: "personless" event. Ingestion uses a deterministic person UUID and skips the Postgres person write [docs].
- **`$lib`, `$lib_version`, `$browser`, `$os`, `$device_type`, `$geoip_*`** (GeoIP is a transformation at ingestion), and **`$ip`**. `ip: false` in posthog-js by default, meaning the client does not send IP; the server derives it, and the project can anonymize it [code].
- **Initial/attribution person properties** set via `$set_once`: `$initial_referrer`, `$initial_utm_source`, and so on [unverified exact list].

### 2.1 Persons and person profiles

`person_profiles` (posthog-js default `'identified_only'`; also `'always'`; mobile also `'never'`) [docs: https://posthog.com/docs/data/anonymous-vs-identified-events]:

- **What creates a profile.**
  - Web: `identify()`, `alias()`, `group()`, `setPersonProperties()`, `setGroupPropertiesForFlags()`. `setPersonPropertiesForFlags()` deliberately does NOT.
  - Mobile: identify/alias/group.
  - Backend SDKs/API: identified by default.
  - Once a profile exists, later events are identified; past events stay anonymous.
- **Anonymous events lose:** person properties, cohorts, lifecycle, person-property flag/survey targeting, the `persons` SQL table, and group analytics.
- **Anonymous events keep:** trends, funnels, event properties, SQL.
- **`createPersonProfile()`** for the cross-domain case. **`reuseAnonymousId: true`** omits `$anon_distinct_id` from `$identify`, so no merge [code: posthog-js types].

Storage:

- Postgres is the source of truth for persons and distinct-id mappings.
- ClickHouse `person` (ReplacingMergeTree, `ver=version`, `ORDER BY (team_id, id)`, columns `id, created_at, team_id, properties, is_identified Int8, is_deleted Int8, version UInt64, last_seen_at`).
- `person_distinct_id2` (ReplacingMergeTree ver=version, `ORDER BY (team_id, distinct_id)`).
- `person_distinct_id_overrides` (same shape; only rows with `version > 0`, i.e. re-pointed distinct_ids; `index_granularity=512`; also exposed as a dictionary using `argMax(person_id, version)`).
- `person_static_cohort` [code: `posthog/models/person/sql.py`].
- Reads use `argMax`/`max(version)` + `is_deleted=0` patterns.

Persons-on-events (PoE) modes, `PersonsOnEventsMode` [code]:

- `disabled`
- `person_id_no_override_properties_on_events`
- `person_id_override_properties_on_events`
- `person_id_override_properties_joined`

The current model:

- `person_id` and `person_properties` are denormalized onto the event at write time.
- After a merge, old events are re-pointed at query time via `person_distinct_id_overrides`, with a periodic squash job [unverified squash details].

New Rust person service "personhog" [code: `rust/personhog-*`, `docker-compose.base.yml`]:

- router / leader / replica / writer / identity.
- vNode ownership with leader handoff.
- In-memory leader cache with a compacted Kafka topic `personhog_updates`, batch-upserted to Postgres by `personhog-writer`.

### 2.2 Identify / alias / merge semantics

From [docs: https://posthog.com/docs/how-posthog-works/ingestion-pipeline], [docs: https://posthog.com/docs/product-analytics/identify].

- **`$identify(distinct_id, $anon_distinct_id)`:**
  - Neither ID is mapped: create a person and map both.
  - Exactly one is mapped: add a mapping for the other to that person.
  - Both are mapped to different persons: merge the anonymous person into the identified one and delete it. The non-anonymous person's property values win on conflict.
- **Guard:** "We do not allow identifying a user that has already been identified with a different distinct ID." The `is_identified` flag on person enforces this: an already-identified person is not merged into another via `$identify`.
  - Re-identifying with the same ID is a no-op (the JS SDK also dedups repeat calls with the same data).
- **Alias rules:**
  - The alias can't be associated with more than one distinct_id.
  - The alias must not have previously been used as the `distinct_id` of an identify/alias call.
  - The UI shows "Merge restrictions" per person.
- **`$merge_dangerously`:** merges regardless, irreversible. Merging B into A adds B's props to A; A's values win.
- **Refused merges** are still ingested (tied to `distinct_id` only), with an ingestion warning.
- **Split IDs:** a UI action to unmerge, with "no guarantee as to how past events will be treated".
- **Illegal distinct IDs:**
  - Empty or whitespace-only.
  - Case-insensitive: `anonymous`, `guest`, `distinctid`, `distinct_id`, `id`, `not_authenticated`, `email`, `undefined`, `true`, `false`.
  - Case-sensitive: `[object Object]`, `NaN`, `None`, `none`, `null`, `0`.
- **Property ordering:** `$set`/`$set_once` apply in ingestion order, not timestamp order. A late-arriving event can overwrite newer values. `$set_once` applies only if the key is absent.
- **`reset()`** on logout makes a new anonymous distinct_id; `reset(true)` also rotates `$device_id`. Without reset, shared devices merge users.
- **Overflowed traffic** (hot distinct_ids, section 8) skips person processing entirely.

---

## 3. Capture API

Routes in the Rust `capture` service (axum) [code: `rust/capture/src/router.rs`]:

- **Analytics:** `/e`, `/i/v0/e`, `/batch`, `/capture`, `/track`, `/engage`, each with and without a trailing slash, each accepting GET and POST.
- **Replay:** `/s` (a separate deployment with `CAPTURE_MODE=recordings`).
- **AI:** `/i/v0/ai`, `/i/v0/ai/batch`, `/i/v0/ai/otel`.
- **Ops:** `/_readiness`, `/_liveness`, `/metrics`.
- There is also a newer `v1` handler module [code: `rust/capture/src/v1/`].
- `POST /flags` is a separate Rust service (`rust/feature-flags`), which also serves `/flags/definitions` [code: `rust/feature-flags/src/api/request_outcome.rs`].

Payloads [docs: https://posthog.com/docs/api/capture]:

- Single: `{api_key, event, distinct_id, properties, timestamp}`.
- Batch: `{api_key, historical_migration?, sent_at?, batch: [RawEvent...]}`.
- The batch body limit is 20MB by default; there is no event-count limit.
- `historical_migration: true` routes to `events_plugin_ingestion_historical`, which keeps ordering and avoids spike detection. Capture can also auto-reroute events older than `historical_rerouting_threshold_days` (default 1) when enabled [code: `config.rs`].

Encoding and compression [code: `rust/capture/src/payload/types.rs`, `decompression.rs`]:

- Query param `compression` = `gzip` | `gzip-js` | `lz64` | `lz-string` | `base64` | `b64`.
- Gzip is also detected by magic bytes.
- LZ64 is lz-string base64 text; decompression is output-capped.
- There is a "best-effort" path for uncompressed or base64 bodies, plus legacy form bodies (`data=`, `compression=` in a form-POST body) and GET `?data=`.
- Decompression-bomb detection uses compression-ratio metrics.
- `/flags?config=true` advertises `supportedCompression: ["gzip","lz64"]` to clients [docs: https://posthog.com/docs/api/flags].
- Query `?beacon=1` returns 204 (for sendBeacon).
- Query `_` / `sent_at` is in ms. Values that look like seconds (year <= 2020) are ignored [code: `EventQuery::sent_at`].

Size limits:

- Kafka producer `message.max.bytes` defaults to 1,000,000 bytes [code: `kafka_producer_message_max_bytes`]. This is effectively a ~1MB per-event cap after serialization.
- The AI lane allows 25MB sum-of-parts (`ai_max_sum_of_parts_bytes`) and has a separate `ai_max_event_bytes` [code].
- posthog-js truncates property strings at `properties_string_max_length: 65535` [code].
- Replay: session IDs over 70 chars, or with characters other than `[A-Za-z0-9-]`, are rejected [docs: https://posthog.com/handbook/engineering/session-replay/session-replay-architecture].

Timestamp / clock-skew algorithm [code: `rust/common/types/src/timestamp.rs`]:

```
if offset present:            ts = now - offset_ms                       (source=offset)
elif timestamp && sent_at:    skew = sent_at - now; ts = timestamp - skew (source=sent_at_skew)
elif timestamp:               ts = timestamp                              (raw)
else:                         ts = now
if ts - now > 23h:            ts = now        (future clamp)
if year<0 || year>9999:       ts = UNIX_EPOCH
```

- `sent_at` is taken from the batch body or `?_=` ms. It is ignored for historical migrations (`ignore_sent_at`).
- Date parsing tries RFC3339, then `dateparser`, then `jiff` civil datetime.
- Node ingestion only re-parses; normalization moved into Rust [code: `nodejs/src/ingestion/common/timestamps.ts`].

Dedup:

- Kafka key is `token:distinct_id`, or `token:ip` in cookieless mode [code: `CapturedEvent::key`].
- Exact duplicates collapse eventually in ClickHouse: `ReplacingMergeTree(ver=_timestamp)` with `ORDER BY (team_id, toDate(timestamp), event, cityHash64(distinct_id), cityHash64(uuid))`. Same uuid + same sort key means dedup on merge, which is not guaranteed at query time.
- A `kafka-deduplicator` Rust service holds per-partition RocksDB state. It dedups on `(timestamp, event_name, distinct_id, token)`, tracks the UUID set and property-similarity scores, optionally publishes duplicates to a topic, and also has a `clickhouse_events` pipeline [code: `rust/kafka-deduplicator/README.md` and `src/pipelines/ingestion_events/mod.rs`].

Rate limiting, quota, overflow [code: `rust/capture/src/config.rs`; docs: ingestion-pipeline]:

- **Billing quota limiting:** a Redis-backed limiter per token and product (`quota_limiters.rs`). Over-quota events are dropped while the client still gets 200 [unverified client response]. `/flags` returns `quotaLimited: ["feature_flags"]` with empty flags.
- **Overflow:** a per-(token, distinct_id) governor (`overflow_per_second_limit` default 100, burst 1000). Docs say a stream sustaining over ~5,000 events/min goes to `events_plugin_ingestion_overflow`. Ordering is relaxed and person processing is skipped. The client still gets 200, and an ingestion warning is logged.
- **Forced overflow** (`ingestion_force_overflow_by_token_distinct_id`) and drop lists (`drop_events_by_token_distinct_id`).
- **Global Redis sliding-window limiter** per (token, distinct_id): default 300k/60s (comment: "too high to trigger limiting in production"). Fail-open design with local caches.
- **"Event restrictions"**, refreshed from Redis.
- **S3 fallback sink** if Kafka is unavailable (`s3_fallback_enabled`).
- **Kafka producer:** linger 20ms, 400MiB queue, 20s message timeout, compression codec configurable (default none).
- **Kafka topics written by capture:** `events_plugin_ingestion`, `_overflow`, `_historical`, `_dlq`, `_ai`, `_ai_overflow`, `session_recording_snapshot_item_events`, `session_recording_snapshot_item_overflow`, `heatmaps_ingestion`, `ingestion-clientwarnings-main-1`, `ingestion-errortracking-main`, `ingestion-traces`, `ingestion-metrics`.

---

## 4. Client SDK surface (posthog-js)

Public methods [code: posthog-js `packages/browser/src/posthog-core.ts`]:

- **Lifecycle and capture:** `init`, `capture`, `identify`, `alias`, `reset`, `register`, `register_once`, `unregister`, `register_for_session`, `unregister_for_session`, `get_property`, `getSessionProperty`.
- **Groups and persons:** `group`, `getGroups`, `resetGroups`, `setPersonProperties`, `unsetPersonProperties`, `createPersonProfile`, `setPersonPropertiesForFlags` / `resetPersonPropertiesForFlags`, `setGroupPropertiesForFlags` / `resetGroupPropertiesForFlags`.
- **Consent:** `opt_in_capturing`, `opt_out_capturing`, `has_opted_in_capturing`, `has_opted_out_capturing`, `clear_opt_in_out_capturing`, `get_explicit_consent_status`, `is_capturing`.
- **Feature flags:** `isFeatureEnabled`, `getFeatureFlag`, `getFeatureFlagPayload`, `getFeatureFlagResult`, `getAllFeatureFlags`, `onFeatureFlags`, `reloadFeatureFlags`, `updateFlags`, `featureFlags.overrideFeatureFlags({flags, payloads} | false)` [code: `packages/node/src/feature-flag-evaluations.ts` comments], `getEarlyAccessFeatures`, `updateEarlyAccessFeatureEnrollment`.
- **Session replay:** `startSessionRecording`, `stopSessionRecording`, `sessionRecordingStarted`, `get_session_replay_url`, `get_session_id`, `onSessionId`.
- **Error tracking:** `captureException`, `startExceptionAutocapture`, `stopExceptionAutocapture`, `addExceptionStep`.
- **Logs and traces:** `captureLog`, `captureTraceMetric`, `captureTraceFeedback`.
- **Surveys:** `getSurveys`, `getActiveMatchingSurveys`, `renderSurvey`, `displaySurvey`, `canRenderSurvey`, `onSurveysLoaded`.
- **Other:** `loadToolbar`, `setInternalOrTestUser`, `set_config`, `on(event, cb)`, `get_distinct_id`, `debug`.

`init` defaults (`defaultConfig`) [code]:

- **Hosts and token:** `api_host: 'https://us.i.posthog.com'` (EU `eu.i.posthog.com`), `ui_host`, `flags_api_host`, `asset_host`, `token`.
- **Capture:** `autocapture: true`, `capture_pageview: true` (or `'history_change'` with defaults >= 2025-05-24), `capture_pageleave: 'if_capture_pageview'`, `rageclick: true`.
- **Persistence:**
  - `persistence: 'localStorage+cookie'` (options also `cookie`, `localStorage`, `sessionStorage`, `memory`); before 1.92.0 the default was `cookie`.
  - `cookie_expiration: 365` (days), `cross_subdomain_cookie` auto, `secure_cookie` if https, `persistence_name`.
  - Cookie is named `ph_<token>_posthog` [unverified].
- **Attribution:** `save_referrer: true`, `save_campaign_params: true`, `custom_campaign_params`.
- **Consent and privacy:** `opt_out_capturing_by_default: false`, `opt_out_capturing_persistence_type: 'localStorage'`, `respect_dnt: false`, `property_denylist`, `mask_all_text: false`, `mask_all_element_attributes: false`, `mask_personal_data_properties: false`, `ip: false`.
- **Feature flags:** `feature_flag_request_timeout_ms: 3000`, `feature_flag_request_max_retries: 1`, `advanced_disable_flags` / `advanced_disable_feature_flags` / `advanced_disable_feature_flags_on_first_load`, `advanced_feature_flags_dedup_per_session`, `evaluation_contexts`, `flag_keys`.
- **Sessions and persons:** `session_idle_timeout_seconds: 1800`, `person_profiles: 'identified_only'`.
- **Hooks:** `before_send` (mutate or drop events), `sanitize_properties`, `get_device_id`, `bootstrap: {}`.
- **Transport:** `disable_compression: false`, `request_batching: true`, `request_queue_config.flush_interval_ms: 3000`, `properties_string_max_length: 65535`.
- **Rate limiting:** `rate_limiting: {events_per_second: 10, events_burst_limit: 10*eps}`, a per-browser token bucket. When it drains, events are dropped and one `$$client_ingestion_warning` is sent [code: `posthog-config.ts`].
- **`defaults: 'YYYY-MM-DD'`:** a date-versioned defaults switch that opts into newer behaviors. Examples: `2025-11-30` gives strictMinimumDuration; `2026-05-30` gives canvas resolutionScale 0.6 and persistence debounce 250ms; `2026-06-25` gives streamNetworkBody; `2026-08-30` gives captureJsonLd. **A good pattern to copy.**
- **Cookieless:** `cookieless_mode: 'always'|'on_reject'`; the project setting is also required.

Autocapture config [code: `posthog-config.ts`]: `url_allowlist`, `url_ignorelist`, `dom_event_allowlist`, `element_allowlist`, `css_selector_allowlist`, `css_selector_ignorelist`, `element_attribute_ignorelist`, `capture_copied_text`. Elements with class `ph-no-capture` are never captured, and `ph-no-deadclick` opts out of dead clicks.

Bootstrap: `{distinctID, isIdentifiedID, featureFlags, featureFlagPayloads, sessionID}`, used for SSR so flags are available synchronously on first paint.

Queue, retry, transport [code: posthog-js `request-queue.ts`, `retry-queue.ts`]:

- Batch flush every 3s.
- Retries apply to non-200 responses, except 4xx (400–499 are not retried).
- Max 10 retries, or 3 for status 0 (network/CORS/ad-block).
- Backoff is `3000 * 2^n` ms, capped at 30 min, with ±50% jitter; a `retry_count` query param is added.
- On page unload the queue is flushed with `navigator.sendBeacon` (`disable_beacon` opts out).
- Session replay buffers separately.

Session recording options [code: `posthog-config.ts`]:

- **Masking:** `maskAllInputs` (default true), `maskInputOptions` (per input type: color, date, email, ..., password), `maskInputFn`, `maskTextSelector`, `maskTextFn`, `maskTextClass`, `maskAllElementAttributes`, `maskAttributeFn`, `blockClass`, `blockSelector`, `ignoreClass`, `maskRegionsFn` (canvas regions).
- **Network:** `recordHeaders` (false), `recordBody` (false), `maskCapturedNetworkRequestFn`, `streamNetworkBody`.
- **Canvas and iframes:** `captureCanvas` / `canvasCapture` (fps 0–12, remote default 4; quality "0.4"), `recordCrossOriginIframes`.
- **Snapshotting:** `full_snapshot_interval_millis` (5 min), `compress_events: true` (partial compression of rrweb events), `session_idle_threshold_ms` (5 min; idle pauses recording without rotating the session), mutation throttler buckets, `slimDOMOptions`, `collectFonts`, `inlineStylesheet`, `captureJsonLd`.
- **Duration and sampling:** `strictMinimumDuration`, `sampleRate`.
- **Console logs:** `enable_recording_console_log` (falls back to the project setting).
- **Server-side (remote config):** sampling rate, minimum duration, linked-flag gating, URL triggers, event triggers, and URL blocklist [unverified exact naming; `trigger_pending_buffer_interval_millis` exists in code].

Mobile SDK offline queueing:

- **iOS** [docs: https://posthog.com/docs/libraries/ios/configuration]: `flushAt 20` (5 on tvOS), `maxQueueSize 1000` (100 on tvOS), `maxBatchSize 50`, `flushIntervalSeconds 30`, `captureApplicationLifecycleEvents true`, `captureScreenViews true`, `personProfiles .identifiedOnly`, `preloadFeatureFlags true`, `sendFeatureFlagEvent true`, `sessionReplay false`. The queue is on disk; the oldest event is dropped when full; it flushes only when online.
- **Android** [docs: https://posthog.com/docs/libraries/android]: `flushAt 20`, `maxQueueSize 1000` (memory + disk, FIFO eviction), `maxBatchSize 50`, `flushIntervalSeconds 30`, `captureDeepLinks true`, `captureApplicationLifecycleEvents true`, `captureScreenViews true`. The queue flushes on restart if online, and `flush()` offline aborts. Requires API 23+.
- **React Native** [docs: https://posthog.com/docs/libraries/react-native]: `flushAt 20`, `flushInterval 10000ms`, `maxBatchSize 100`, `maxQueueSize 1000`, `fetchRetryCount 3`, `fetchRetryDelay 3000`, `requestTimeout 10000`, `featureFlagsRequestTimeoutMs 10000`, `persistence 'file'` (or `'memory'`, `customStorage`/`customAsyncStorage`), `captureAppLifecycleEvents true`, `sessionExpirationTimeSeconds 1800`, `enablePersistSessionIdAcrossRestart false`, `enableSessionReplay false`.
- **Flutter:** wraps the native SDKs [unverified details].

---

## 5. Feature flags

### Endpoint

Source: [docs: https://posthog.com/docs/api/flags].

- **Request:** `POST /flags?v=2[&config=true]` with body `{api_key|token, distinct_id, groups?: {type: key}, person_properties?, group_properties?: {type: {...}}, evaluation_contexts? (legacy evaluation_environments), $anon_distinct_id?, flag_keys_to_evaluate?, ...}`. The last two fields are [unverified]; posthog-js does send `$anon_distinct_id` on identify, which is what reuseAnonymousId suppresses [code].
- **v2 response:**
  ```json
  {
  	"flags": {
  		"my-flag": {
  			"key": "my-flag",
  			"enabled": true,
  			"variant": "test",
  			"reason": {
  				"code": "condition_match",
  				"condition_index": 0,
  				"description": "Matched condition set 1"
  			},
  			"metadata": { "id": 42, "version": 3, "payload": "{\"a\":1}" }
  		}
  	},
  	"errorsWhileComputingFlags": false,
  	"requestId": "...",
  	"quotaLimited": ["feature_flags"] // only when over quota; flags then empty
  }
  ```
  - `config=true` adds `config` (e.g., `enable_collect_everything`), `toolbarParams`, `isAuthenticated`, `supportedCompression`, plus remote config for replay/surveys/heatmaps/etc.
  - `errorsWhileComputingFlags: true` allows clients to do a partial update (keep cached values for failed flags).
  - Holdout users get variant `holdout-<id>`.
  - Runtime (client vs server) is inferred from User-Agent, Origin, Referer, and Sec-Fetch-*, and filters which flags return.
  - An `X-Forwarded-For` override changes GeoIP for evaluation.
- **Legacy `/decide`** (`v=3` returned `{featureFlags:{key: bool|variant}, featureFlagPayloads:{key: json-string}, errorsWhileComputingFlags, sessionRecording, ...}`; `v=4` introduced the per-flag `{enabled, variant, reason, metadata}` shape that `/flags?v=2` keeps) [unverified]. posthog-js now always uses `/flags/?v=2`, and `advanced_disable_decide` is a deprecated alias [code].
- **Reason codes** seen: `condition_match`, `no_condition_match`. Others such as `out_of_rollout_bound`, `no_group_type`, and `holdout_condition_value` are [unverified]. Python's `ConditionMatch` has MATCH / NO_MATCH / OUT_OF_ROLLOUT_BOUND, "mirroring the server-side (Rust) engine" [code].
- **Health:** `/flags` reports degraded (`persons_db_failed`), rate_limited, timed_out, and panicked outcomes [code: `rust/feature-flags/src/api/request_outcome.rs`].

### Flag types and conditions

Source: [docs: https://posthog.com/docs/feature-flags/creating-feature-flags].

- **Boolean:** false when disabled; true when a condition matches; null/undefined when none match.
- **Multivariate:** variants with rollout percentages that sum to 100, plus per-variant payloads.
- **Remote config:** always returns the payload; no conditions.
- **Payloads:** any JSON. Encrypted payloads exist and are only decrypted with a personal API key in local eval [docs: local-evaluation].
- **Condition sets (`filters.groups[]`):** OR across sets; within a set, all property filters AND, then the rollout %. Each set can carry a `variant` override.
  - An optional "stop at first matching set" (`early_exit`): if the filters match but rollout excludes the user, return false.
  - Decimal rollouts down to 0.01%.
- **Targeting:**
  - Person properties, cohorts, group properties (`aggregation_group_type_index` at flag or condition level, allowing mixed targeting), GeoIP, and warehouse-mapped properties.
  - Operators: `exact, is_not, is_set, is_not_set, icontains, not_icontains, regex, not_regex, starts_with, not_starts_with, ends_with, not_ends_with, gt, gte, lt, lte, is_date_before, is_date_after, semver_eq/neq/gt/gte/lt/lte, semver_tilde/caret/wildcard, flag_evaluates_to` (the local-eval set) [code: posthog-python `feature_flags.py`]. Server-side also: `is_date_exact, between, not_between, min, max, in, not_in, icontains_multi` [code: `PropertyOperator` enum].
  - Relative dates like `-7d` [unverified].
- **Bucketing identifier:** `distinct_id` (default) or `device_id` ("Device" targeting mode, for anonymous stability). For group flags, the bucketing value is the group key.
- **Persist across authentication ("experience continuity"):** keeps the anon value after identify by storing hash-key overrides in Postgres. Requires `person_profiles: 'always'`, is slower, and is incompatible with local eval and bootstrapping.
- **Evaluation runtime:** client, server, or both. **Evaluation contexts:** tags that must match the request's `evaluation_contexts`; untagged flags always evaluate.
- **Flag dependencies:** `type: "flag"` property with operator `flag_evaluates_to` and value `true` / `false` / a variant string.
  - Any variant matches `true`. Variants are case-sensitive.
  - A `dependency_chain` gives evaluation order.
  - A flag can't be deleted while others depend on it.
- **Holdouts** (`filters.holdout = {id, exclusion_percentage}`): evaluated BEFORE release conditions.

### Hashing algorithm

Exact source: [code: posthog-python `posthog/feature_flags.py`]. The Rust server and every SDK must match.

```python
LONG_SCALE = float(0xFFFFFFFFFFFFFFF)        # 15 hex F's = 2^60 - 1
def _hash(key, bucketing_value, salt="", separator="."):
    hash_key = f"{key}{separator}{bucketing_value}{salt}"
    return int(sha1(hash_key.encode("utf-8")).hexdigest()[:15], 16) / LONG_SCALE

# rollout: user is IN if  _hash(flag_key, bucketing_value) <= rollout_pct/100
#   (code: excluded if hash > pct/100)
# variant: h = _hash(flag_key, bucketing_value, salt="variant")
#   walk variants in order; value_min=0; value_max = value_min + pct/100;
#   pick first variant with value_min <= h < value_max
# holdout: _hash("holdout", bucketing_value, separator="-")  -> sha1("holdout-<value>")
#   held out if hash <= exclusion_pct/100 (100% = everyone, no hash) -> variant "holdout-<id>"
```

- Rollout and variant use the same key but different salt (`""` vs `"variant"`), so rollout membership and variant are independent.
- Changing a flag key re-buckets everyone.
- Rollout order inside a condition: filters first, then hash.

### Evaluation order

Per flag [code]:

1. `feature_enrollment` (early access), using person prop `$feature_enrollment/<key>`.
2. Holdout.
3. Condition sets in order. The first match returns the condition's variant override or the hashed variant, else `true`.
4. With `early_exit`, an OUT_OF_ROLLOUT_BOUND result stops evaluation.

### Local evaluation (server SDKs)

Source: [docs: https://posthog.com/docs/feature-flags/local-evaluation].

- Poll `GET /flags/definitions` (formerly `/api/feature_flag/local_evaluation`) every 30s (Go: 5 min), with ETag / 304.
- Auth uses a project secret key with scope `feature_flag:read`. Older options are "personal API key" and a deprecated "feature flags secure API key".
- Definitions carry `version`, which bumps when the flag or a referenced cohort changes. Cohorts are shipped inline. A group type mapping is included.
- **Cannot evaluate locally:**
  - Experience continuity.
  - Early-access-linked flags.
  - Static cohorts (`RequiresServerEvaluation`, which triggers immediate fallback).
  - `is_not_set` conditions (you can only check what you pass).
  - `early_exit` in most SDKs.
  - Complex dynamic cohorts in older SDKs.
- Missing properties raise `InconclusiveMatchError` and fall back to `/flags` unless `onlyEvaluateLocally`.
- Unresolved-reason precedence: EXPERIENCE_CONTINUITY > UNSUPPORTED_DEFINITION > UNRESOLVED_DEPENDENCY > MISSING_CONTEXT [code].
- Serverless gotcha: a per-invocation client loses its cache, so use a module-level client or an external cache. PHP has no poller.

### Telemetry and SSR

- `$feature_flag_called` is emitted on first access per flag+value. The JS SDK dedups per page/session (`advanced_feature_flags_dedup_per_session`). Server SDKs dedup in memory. Mobile has `sendFeatureFlagEvent`.
- Experiments use these events (or a custom exposure event) as exposures.
- SSR: evaluate on the server (local eval), pass a `bootstrap` object to posthog-js, and the client calls `/flags` afterwards to refresh.

---

## 6. Experiments

Current engine [docs: https://posthog.com/docs/experiments/statistics; code: `products/experiments/stats/{bayesian,frequentist,shared}`]:

**Metrics**

- Kinds: funnel (conversion), mean (count/sum/numeric per user), ratio (numerator/denominator, delta method), and retention (analyzed as a ratio).
- Data-warehouse metrics are supported.
- Effect size defaults to relative: `(t_mean - c_mean)/c_mean`.

**Bayesian (default)**

- Gaussian approximation on the effect. Default is a non-informative prior (`prior_mean=0`, `prior_variance=1`, `proper_prior=False`); an informative prior is optional [code: `stats/bayesian/method.py`].
- Funnel variance is `p(1-p)/n`; means use sample variance from sum and sum-of-squares; ratios use the delta method, valid when the denominator CV is below 0.3.
- Outputs:
  - "Chance to win" = normal CDF (flipped for lower-is-better metrics).
  - 95% credible interval = mean ± 1.96 sd.
  - Significant if chance > confidence level or < 1 − level. Levels are 90/95 (default)/99%.

**Frequentist** [docs: https://posthog.com/docs/experiments/statistics-frequentist]

- Two-sided Welch t-test with Welch–Satterthwaite df; `p = 2(1 - t_cdf(|t|, df))`.
- Relative-difference variance via the delta method: `Vt/(nt·μc²) + μt²·Vc/(nc·μc⁴)`.
- CI = estimate ± t_crit·sqrt(var).
- Optional **sequential testing** (always-valid p-values and confidence sequences, Waudby-Smith et al. 2023) with a tuning parameter defaulting to 5,000 (≈ expected sample size).

**Validation gates (both engines)**

- At least 50 exposures per variant.
- Funnels: at least 5 conversions per variant and n·p > 5, n·(1-p) > 5.
- The control mean must be non-zero for mean/ratio metrics.

**Variance reduction and outliers**

- **Winsorization:** optional percentile clamping per metric (e.g., p1/p99); ratio numerator and denominator are capped independently.
- **CUPED** [docs: https://posthog.com/docs/experiments/cuped]: pre-exposure lookback defaults to 14 days (range 1–30). Applies to mean metrics (not outcome-threshold or session-property ones) and ordered funnels without warehouse steps. Not applied to ratio, retention, or unordered funnels. Project-level default with per-experiment override.

**Multiple comparisons**

- None corrected: each variant vs control and each metric is tested independently.
- The docs quantify it: ~23% chance of at least one false positive with 5 metrics, ~40% with 10.

**SRM**

- Chi-square goodness-of-fit of observed exposures vs expected split; flagged when `p < 0.001` [code: `products/experiments/backend/health/checks/srm.py`, `SAMPLE_RATIO_MISMATCH_P_VALUE = 0.001`, `MATERIAL_SHARE_OFF_SPLIT = 0.01`].
- It's part of a health-check registry that also covers bucketing_changed, split_changed, conditions_changed, forced_variant, shared_flag, early_significance, metrics_not_matured, zero_exposures, and new_flag_no_traffic, among others [code].

**Legacy engine** (pre-2025) [docs: https://posthog.com/docs/experiments/legacy-methodology; secondary previews]

- Funnels used Beta(α=1, β=1 prior) with Monte Carlo (10k–100k draws) for "probability to be best", and significance on **expected loss below 1%** (VWO SmartStats).
- Count trends used Gamma-Poisson (e.g., `Gamma(count+1, exposure+1)`), originally with a Poisson-means p<0.05 test.
- Worth copying for a simple v1 engine.

**Other features**

- Exposure: the default exposure event is `$feature_flag_called` with matching flag+variant; custom exposure events and "first exposure" semantics are supported. Users seeing multiple variants are excluded or handled [unverified specifics].
- Primary and secondary metrics, shared metrics library, holdouts (global % excluded across experiments; `holdout-<id>` variant), sample-size / running-time calculator using MDE (minimum detectable effect) [unverified formula; likely the standard `n ≈ 16σ²/δ²` rule of thumb], and pre-aggregated exposure tables (`experiment_exposures_preaggregated`) [code: HogQL schema list].

---

## 7. Session replay

**Client (web)**

- rrweb record: an initial FullSnapshot (type 2) plus Meta (type 4), then IncrementalSnapshot (type 3) events.
- Incremental sources are Mutation 0, MouseMove 1, MouseInteraction 2, Scroll 3, ViewportResize 4, Input 5, TouchMove 6, MediaInteraction 7, StyleSheetRule 8, CanvasMutation 9, Font 10, Log 11, Drag 12, StyleDeclaration 13, Selection 14, AdoptedStyleSheet 15. Custom (type 5) and Plugin (type 6) carry console logs and network data [unverified: rrweb enum numbering from prior knowledge].
- Full snapshot every 5 min while active.
- Partial event compression (`compress_events`).
- Idle threshold 5 min pauses recording.
- A mutation throttler (token buckets on count and bytes) protects against DOM storms.
- Batches go out as `$snapshot` events with `$snapshot_items[]`, `$session_id`, `$window_id`, `$snapshot_source` [docs: handbook replay architecture].

**Privacy defaults**

- Web [docs: https://posthog.com/docs/session-replay/privacy]: inputs masked by default; general text NOT masked by default (`maskTextSelector: "*"` masks all); `ph-no-capture` replaces an element with a same-size block; `maskCapturedNetworkRequestFn` also redacts URLs; some headers are scrubbed automatically.
- iOS: `accessibilityIdentifier=ph-no-capture` masks, `ph-no-mask` unmasks; SecureField and email/password text fields are always masked.
- Android: tag/contentDescription `ph-no-capture` / `ph-no-mask`.
- React Native: `maskAllTextInputs`, `maskAllImages`, `maskAllSandboxedViews` all default true.
- Flutter: `maskAllTexts`, `maskAllImages`, `maskAllPlatformViews` true, `captureTouches` true (consider disabling for PIN pads).

**Mobile approach** [docs: https://posthog.com/docs/session-replay/mobile]

- iOS/Android default to **wireframe mode**: the native view hierarchy is serialized to JSON and rendered as HTML wireframes in the player.
- Optional **screenshot mode** (`screenshot` / `screenshotMode`).
- React Native and Flutter always use screenshots.
- Captures touches, network requests, and logs (Android Logcat option).

**Ingestion** [docs: https://posthog.com/handbook/engineering/session-replay/session-replay-architecture]

- **Capture:** a separate Rust capture deployment (`CAPTURE_MODE=recordings`, route `/s/`) validates session IDs and writes to Kafka `session_recording_snapshot_item_events`. Sessions that are billing-limited, or listed in Redis key `@posthog/capture-overflow/replay`, go to `session_recording_snapshot_item_overflow`, keyed on session_id.
- **Blob ingestion consumer (Node):**
  - `SessionRecordingIngester` → `SessionBatchRecorder` → `SnappySessionRecorder`.
  - Each session's events are buffered as JSONL `[windowId, event]` lines and the block is Snappy-compressed.
  - Buffers flush at 10s max age or 100MB.
  - Batches are written as S3 multipart objects containing many session blocks, addressed by byte range (`s3://bucket/key?range=bytes=start-end`).
  - The prefix varies by retention period: `session_recordings/{team_id}/{session_id}/...` for persisted LTS copies, `{prefix}/{timestamp}-{suffix}` for batch files.
  - It also counts clicks, keypresses, mouse activity, console logs, URLs, and active time.
- **Metadata:** Kafka `clickhouse_session_replay_events` → ClickHouse `session_replay_events` (AggregatingMergeTree, sharded).
  - Per session: first/last timestamps, block locations and first/last ts/URL per block, first_url, all_urls, click/keypress/mouse/console counts, active_milliseconds, size, message_count, event_count, snapshot_source/library, retention_period_days (TTL).
  - The legacy `session_recording_events` table stored raw snapshots in ClickHouse and is deprecated.
- **Playback:** `GET .../session_recordings/:id/snapshots` lists sources (`["blob"]`), then `?source=blob` returns pre-signed S3 URLs (60s expiry). The browser downloads blocks, decompresses Snappy, parses JSONL, and feeds rrweb-player.
- **LTS:** recordings 24h–90d old are swept and copied to long-term storage; pinned/playlisted ones persist.
- **Other services:** `replay-anonymizer`, and a recording rasterizer (Dockerfile) for video export / AI.

**Retention by plan** [docs: https://posthog.com/docs/session-replay/data-retention]

- Free up to 30 days, pay-as-you-go up to 90 days, Boost/Scale up to 1 year, Enterprise up to 5 years.
- Retention is fixed per recording at capture time.
- JSON export and re-import are available.

**Controls:** sampling (`sampleRate`), minimum duration (`strictMinimumDuration` measures actual buffered data), linked flag, URL/event triggers, canvas, console, network (`recordHeaders`/`recordBody` off by default).

**Lesson:** a 2022 investigation of missing recordings found ad blockers and client issues [docs: https://posthog.com/blog/missing-recordings].

---

## 8. Ingestion architecture

### Pipeline

Sources: [docs: https://posthog.com/docs/how-posthog-works/ingestion-pipeline] and [code].

```
SDK → capture (Rust/axum) → Kafka events_plugin_ingestion{,_overflow,_historical,_ai}
    → (new) rust ingestion-consumer → gRPC ordered per-key streams → Node ingestion workers
    → [1] CDP transformations (Hog functions; only chance to mutate/drop before storage; GeoIP etc.)
    → [2] person processing (Postgres / personhog; skipped if $process_person_profile=false or overflow)
    → [3] event processing (team/token resolution, group properties, IP anonymization, heatmap extraction)
    → [4] produce clickhouse_events_json (event + person_id/properties + group props)
    → [5] CDP realtime destinations consume (events immutable now)
ClickHouse: Kafka engine table (kafka_events_json) → MV → writable_events (Distributed, sipHash64(distinct_id)) → sharded_events
```

- **`ingestion-consumer` (Rust)** [code: `rust/ingestion-consumer/README.md`]: groups Kafka messages by key, keeps at most one in-flight request per key to preserve per-distinct-id order, uses an offset ledger for contiguous commits, and drains workers gracefully. "Ordering sentinel" metrics count violations.
- **Partitioning:** the Kafka key is `token:distinct_id`, which is how per-person ordering for person updates is achieved. Overflow nulls the key (or preserves it with `overflow_preserve_partition_locality`).
- **`property-defs-rs`** consumes `team_event_partitioned_events_json`, which a **WarpStream Bento** pipeline repartitions by team from `clickhouse_events_json`. It writes event/property definitions to Postgres with dedup caches [code]. So production uses WarpStream as well as (or instead of) MSK; job posts mention "WarpStream as a fallback for MSK in capture" [unverified, secondary].
- **DLQ:** `events_plugin_ingestion_dlq` with headers `dlq_reason`, `dlq_step`, `dlq_timestamp` [code].
- **Ingestion warnings:** written to a ClickHouse `ingestion_warnings` table via Kafka (capture can emit them too, `capture_ingestion_warnings_*`) and surfaced in the "health checks" UI. Examples: distinct_id truncated, illegal distinct id, refused merge, `ignored_invalid_timestamp`, event overflowed, client rate-limited [code: `nodejs/src/ingestion/common/timestamps.ts`; docs].
- **Other topics:** `clickhouse_session_replay_events`, `clickhouse_ai_events_json`, person and distinct-id topics into ClickHouse (`clickhouse_person`, `clickhouse_person_distinct_id` [unverified exact names]), `personhog_updates` (compacted).

### ClickHouse layout

- **Events:** `sharded_events` is `ReplicatedReplacingMergeTree(ver=_timestamp)`, `PARTITION BY toYYYYMM(timestamp)`, `ORDER BY (team_id, toDate(timestamp), event, cityHash64(distinct_id), cityHash64(uuid))`, `SAMPLE BY cityHash64(distinct_id)`. Wrapped by a `Distributed` table `events` sharded on `sipHash64(distinct_id)` [code].
- **Properties:** stored as a JSON string (ZSTD 3). Hot properties are materialized as `mat_*` columns (auto-materialization by a column materializer), plus the `dmat_string_*` per-team slots and "property groups" (map columns for groups of keys). The newer `events_json` table uses native JSON with typed subcolumns.
- **Persons:** `person` / `person_distinct_id2` / `person_distinct_id_overrides` (ReplacingMergeTree on version). The older `person_distinct_id` uses CollapsingMergeTree(`_sign`) [code].
- **Sessions:** `raw_sessions` v3 is AggregatingMergeTree with `session_id_v7 UInt128` and `session_timestamp` derived from the UUIDv7's top 48 bits. Columns: `argMax(distinct_id)`, `groupUniqArray(distinct_ids)`, min/max timestamp, urls, `argMin` entry_url / `argMax` end_url, device/browser/OS argMin, geoip argMin, entry UTMs/gclid/fbclid/gad_source, channel-type tuple, and `uniq` counts of pageviews/autocaptures/screens. `uniq` is used so re-inserts are idempotent. `PARTITION BY toYYYYMM(session_timestamp)` [code: `posthog/models/raw_sessions/sessions_v3.py`].
- **Others:** `session_replay_events`, `heatmaps`, `ingestion_warnings`, `log_entries`, app metrics, `person_static_cohort`, `cohortpeople`, error-tracking tables, logs/spans/metrics tables, and many web-analytics pre-aggregated tables (`web_stats_preaggregated`, `web_overview_preaggregated`, `web_sessions_preaggregated`, ...) [code: `posthog/hogql/database/schema/`].
- **Funnels as a UDF:** funnels are computed by a **Rust executable UDF** (`aggregate_funnel_vN`) shipped to ClickHouse nodes as `user_scripts`. It is versioned and deployed separately from app code [code: `funnel-udf/README.md`].

### Postgres, Redis, other stores

- **Postgres:** Django app DB (teams, flags, cohorts, insights, dashboards, experiments, surveys, hog functions), plus a separate persons DB (persons, distinct-ids, cohort membership, flag hash-key overrides) [code: `rust/persons_migrations`].
- **Redis/Valkey:** quota/billing limits, overflow lists, rate limiters, flag definition caches ("hypercache"), session-replay overflow keys, Celery.
- **Temporal:** batch exports and workflows.
- **Object storage:** S3/MinIO/SeaweedFS for recordings, exports, and the warehouse.

### Hobby / self-host

Sources: [docs: https://posthog.com/docs/self-host], [code: `docker-compose.base.yml`].

- **Requirements:** "4 vCPU, 16GB RAM, >30GB storage" (Hetzner-equivalent), Ubuntu, a domain A record, and Caddy + LetsEncrypt. posthog-foss README says 4GB memory and "approximately 100k events per month" [docs: https://github.com/PostHog/posthog-foss].
- **Licensing:** MIT. The `ee/` directory is separately licensed. **posthog-foss** is an automated read-only mirror "purged of all proprietary code" (announced on HN Jul 2026) [docs: https://news.ycombinator.com/item?id=48846293].
- **Official stance:**
  - Self-hosted is "officially unsupported", with no paid support. All paid-plan features are Cloud-only.
  - Kubernetes/Helm was sunset in Feb 2023: only ~3.5% of users but a disproportionate infra-team burden; security updates until May 2024; self-hosted licenses discontinued [docs: https://posthog.com/blog/sunsetting-helm-support-posthog].
  - On cost vs a DevOps engineer: "We've literally never seen this math work out."
  - No CVEs are published and versions aren't tagged; "always run the latest image".
- **Services in the current compose base:**
  - Caddy, Postgres 15, Redis 7.2 and Valkey 8.1, a custom ClickHouse image (26.9), ZooKeeper 3.7, Redpanda (Kafka API), SeaweedFS (object store; MinIO previously), Azurite.
  - capture (×3 modes), capture-logs, property-defs-rs, feature-flags, usage-ingestion, personhog replica/router/leader/writer/identity, hypercache-server, livestream.
  - Elasticsearch 7, Temporal (+UI), OpenSearch, OTel collector, Jaeger, DynamoDB-local, duckgres.
  - Plus Django web/worker and Node plugin-server [code].
  - **The dependency count keeps growing, which is the key pain point.**

### Throughput figures

- Job posts say "10s of billions of events a month" and work to "support 4x ingestion throughput" and "harden the persons Postgres database" [unverified, secondary: https://www.workatastartup.com/jobs/81334, https://archive.posthog.com/careers/backend-engineer-ingestion].
- No authoritative per-second numbers were found.

---

## 9. HogQL

**What it is**

- "A wrapper around ClickHouse SQL" with simplified property access and null handling, used for SQL insights, expressions in filters/breakdowns, and the API [docs: https://posthog.com/docs/sql].
- Query API: `POST /api/projects/:id/query` with `{"query": {"kind": "HogQLQuery", "query": "select ..."}}`. Returns `results`, `columns`, `types`, `hogql` (expanded), and `clickhouse` (generated SQL).

**Grammar**

- Derived from ClickHouse's ANTLR grammar with everything except SELECT removed. Placeholders `{name}` are supported; unsupported ClickHouse features raise errors [code: `posthog/hogql/grammar/README.md`].
- Includes CTEs, joins, subqueries, window functions, `ARRAY JOIN`, lambdas, and HogQLX (JSX-like tags) [code: `hogqlx.py`].
- In 2026 the ANTLR C++ parser was replaced by a hand-written Rust recursive-descent + Pratt parser (~16K LOC): ~70x faster in a benchmark, 454x on average in production, verified by differential fuzzing and shadow traffic with zero divergences [docs: https://posthog.com/blog/sql-parser].
- Parser backends: `cpp-json`, `rust-json`, `rust-py` [code: constants].

**Pipeline**

- parse → AST → resolver (binds fields to a typed `Database` schema of virtual tables) → transforms (property access, lazy joins, person-on-events overrides) → printer per dialect.
- Dialects: clickhouse, hogql, postgres, duckdb, mysql, snowflake, redshift, trino [code: `posthog/hogql/printer/`, `HogQLDialect`].
- `properties.$browser` becomes `JSONExtract`/materialized columns; `person.properties.x` becomes person-on-events columns or a lazy join to persons via `person_distinct_ids`.
- Virtual tables include `events`, `persons`, `person_distinct_ids`, `person_distinct_id_overrides`, `sessions` (v1/v2/v3), `groups`, `session_replay_events`, `heatmaps`, `cohort_people`, `static_cohort_people`, `log_entries`, `logs`, `spans`, `numbers`, `system.*`, warehouse tables, and saved views [code: `posthog/hogql/database/schema/`].

**Safety**

- **Tenant guard:** `team_id` is a reserved keyword. The printer's `_ensure_team_id_where_clause` adds `and(team_id = <ctx.team_id>, …)` on **every selected table**. Full SELECTs are refused without `context.team_id`. `team_id` guards are pushed into `postgresql()` external joins too [code: `posthog/hogql/printer/clickhouse.py`].
- **Functions:** only allow-listed ClickHouse functions map through (`posthog/hogql/functions/mapping.py`); PostHog-specific functions and UDFs are added.
- **ClickHouse settings per query:** `readonly=2`, `max_execution_time=60`, `max_ast_elements=4,000,000`, `max_expanded_ast_elements=4,000,000`, `transform_null_in=1`, optional `max_bytes_to_read`, `max_memory_usage` from cloud config [code: constants].
- **Row limits:** default `LIMIT 100`, max 50,000 (`MAX_SELECT_RETURNED_ROWS`). Exports 300k (CSV), retention 100k, heatmaps 1M, cohort calc 1B, notebook materialize 500k, PostHog AI 500 [code].
- Query cost estimation (`hogql/cost`), workloads routing (`workload.py`), access control (`ACCESS_CONTROL.md`, `printer/access_control.py`), and restricted properties.
- **History:** the Dec 2025 ZDI disclosures were an SSRF, a ClickHouse SQL-escaping 0-day, and default Postgres creds leading to an RCE chain (ZDI-25-096/097/099) [docs: https://mdisec.com/inside-posthog-how-ssrf-a-clickhouse-sql-escaping-0day-and-default-postgresql-credentials-formed-an-rce-chain-zdi-25-099-zdi-25-097-zdi-25-096/, HN https://news.ycombinator.com/item?id=46305321]. **The lesson for us: escaping inside a multi-tenant SQL layer is a critical attack surface.**

**Query node kinds** (`NodeKind`) [code: `posthog/schema_enums.py`]

- Data sources: `EventsNode`, `ActionsNode`, `DataWarehouseNode`, `PersonsNode`, `GroupNode`.
- Insights: `TrendsQuery`, `FunnelsQuery`, `RetentionQuery`, `PathsQuery` / `PathsV2Query`, `StickinessQuery`, `LifecycleQuery`, `CalendarHeatmapQuery`.
- Raw queries: `HogQLQuery`, `HogQuery`, `EventsQuery`, `ActorsQuery`, `InsightActorsQuery` (click-through to people), `FunnelsActorsQuery`, `StickinessActorsQuery`, `GroupsQuery`, `SessionsQuery`, `SessionsTimelineQuery`, `RecordingsQuery`, `SessionAttributionExplorerQuery`.
- Web: `WebOverviewQuery`, `WebStatsTableQuery`, `WebGoalsQuery`, `WebVitalsQuery`, `WebVitalsPathBreakdownQuery`, `WebExternalClicksTableQuery`, `WebBotsTableQuery`, and more.
- Experiments: `ExperimentQuery`, `ExperimentExposureQuery`, `ExperimentMetric`, and legacy `ExperimentTrendsQuery` / `ExperimentFunnelsQuery`.
- Error tracking (`ErrorTrackingQuery`, ...), logs/traces/metrics, marketing analytics, revenue, taxonomy (`EventTaxonomyQuery`, `TeamTaxonomyQuery`, `ActorsPropertyTaxonomyQuery`), and wrappers `InsightVizNode`, `DataTableNode`, `DataVisualizationNode`, `SavedInsightNode`.
- Each is a JSON object with `kind` plus typed fields (e.g., `TrendsQuery{series[], interval, dateRange, breakdownFilter, trendsFilter, properties, filterTestAccounts, compareFilter}`). A "query runner" per kind compiles to a HogQL AST and caches results.
- **Recommendation: copy this typed-query design.**

---

## 10. Funnel, retention, and path semantics

### Funnels

Sources: [docs: https://posthog.com/docs/product-analytics/funnels], [code].

- **Step order** (`StepOrderValue`):
  - `ordered` (sequential; any events allowed between steps, default).
  - `strict` (next step must be the immediately next event of the actor).
  - `unordered` (any order).
  - Repeating an event across steps requires repeat occurrences.
- **Conversion window:** `funnelWindowInterval` default **14**, unit default **day**. Units: second, minute, hour, day, week, month [code: `funnel_query_context.py`]. Measured from the first step. The date range applies to the entry step [docs: "date range applies to every step" — ambiguous; verify].
- **Aggregation:** unique persons (default), groups (`aggregation_group_type_index`), or a HogQL expression (e.g., sessions via `properties.$session_id`).
- **Step math:** `total`, `first_time_for_user` (first-ever occurrence must match filters and range), `first_time_for_user_with_filters`.
- **Breakdown attribution** (`BreakdownAttributionType`): `first_touch`, `last_touch`, `all_events` (an actor counted in each value), `step` (value at step N; actors without it excluded).
- **Exclusion steps:** actors who perform an exclusion event between steps i and j (within the window) are removed from the whole funnel.
- **Optional steps:** they don't change required-step counts (per the UDF design note).
- **Viz:** steps (overall vs relative-to-previous conversion), `time_to_convert` (histogram bins), `trends` (conversion over time by entry date; "hide incomplete periods"), `flow`.
- **Correlation analysis** (events/properties correlated with success, odds ratios) [docs]. It hits memory limits at scale [code issue: https://github.com/PostHog/posthog/issues/35668].
- **Implementation:** a ClickHouse executable UDF in Rust (`funnel-udf`, with ordered/unordered steps and trends modules) receives each actor's sorted event array and returns per-step results. Data-warehouse steps are supported.

### Retention

Source: [docs: https://posthog.com/docs/product-analytics/retention].

- **Types:** `retention_first_time` (cohort = actors whose first start event in the date range falls in the period), `retention_recurring` (anyone doing the start event in the period), `retention_first_ever_occurrence` (first-ever, across all time).
- **Periods:** hour, day, week (default), month, with N periods.
- **Return criteria:** "On" (exact interval, default) or "On or after" (cumulative/unbounded).
- **Aggregation:** persons or groups. Breakdowns require the start and return events to share the breakdown value.
- **Mean:** weighted (default) or simple. Reference: Period 0 (default) or the previous period.
- Retention queries allow up to 100k rows [code].

### Paths

Source: [docs: https://posthog.com/docs/product-analytics/paths].

- **Path types:** `$pageview` (by `$current_url`), `$screen` (by `$screen_name`), custom events (by name), and a HogQL expression; they can be combined.
- **Start point and/or end point; exclusions.**
- **Wildcard groups** (`/product/*`).
- **Path cleaning rules:** regex → alias (e.g., `/user/\d+` → `/user/:id`). Project-global rules apply by default, with a toggle. The `is_cleaned_path_exact` operator exists [code].
- **Session splitting:** events more than 30 min apart start a new path. Step count is configurable; min/max people per path.
- Returns the top 50 transitions (edges) by default [docs].
- **Funnel ↔ paths:** "paths after step X / before step Y / between steps" and dropoff paths, via `funnelPaths` [unverified naming], plus "View funnel" from a node.

### Lifecycle and stickiness

Covered in section 1.

---

## 11. Web analytics

Source: [docs: https://posthog.com/docs/web-analytics/dashboard].

- **Overview tiles:** Visitors (unique persons/devices), Page views, Sessions, Avg session duration, Bounce rate, each vs the previous period. Conversions and conversion rate appear when a goal is set.
- **Bounce:** "a session where the user only had one pageview, no autocaptures, and spent less than 10 seconds on the page". The duration threshold is configurable. Requires `$pageleave` and `$autocapture`. Count modes (`BounceRatePageViewMode`): `count_pageviews`, `uniq_urls`, `uniq_page_screen_autocaptures` [code].
- **Paths:** top paths (views, visitors, bounce rate, scroll depth), entry paths, end/exit paths, outbound clicks (`last_external_click_url` in sessions).
- **Sources:** referrers, channel types, UTM source/medium/campaign/content/term.
  - ~20 default channel types: Direct, Paid Search, Organic Search, Paid Social, Organic Social, Referral, Email, Display, SMS, Audio, Push, AI, Other, and more. Computed from entry UTMs, referring domain, gclid/fbclid/gad_source (the `entry_channel_type_properties` tuple in sessions).
  - Custom channel rules (first match wins).
  - A session attribution explorer.
- **Devices:** device type, browser, OS (also as filters), viewport.
- **Geo:** country, region, city, timezone, language (from GeoIP at ingestion).
- **Other tiles:** retention (weekly), active hours heatmap (hour × weekday, UTC by default), goals (actions/custom events), bots/agents table, page directory, notable changes [code: query kinds].
- **Web vitals:** LCP p75 on the first pageview per session (good <2.5s, poor >4s) is what the dashboard docs show. The `$web_vitals` event also covers CLS, INP, and FCP, with percentile choices p75/p90/p99 [unverified for the non-LCP parts].
- **Performance:** web analytics reads the `sessions` table (pre-aggregated per-session rows) and pre-aggregated web tables (`web_stats_preaggregated`, `web_overview_preaggregated`, `web_sessions_preaggregated`, `web_vitals_paths_preaggregated`, `web_goals_preaggregated`) [code]. This is the key to making it fast.
- **Filters:** browser, pathname, device type, country, UTMs, "filter test accounts", and host/domain.

---

## 12. Common complaints and pain points

1. **Self-hosting is hard and unsupported.**
   - Hobby deploy pulls `master` and breaks; users pin specific commits. In issue #28804 (Feb 2025) a user spent many hours, had to pin commit `579408c7...`, and add `property-defs-rs` to compose to make the Events tab work [https://github.com/PostHog/posthog/issues/28804]. Also #29706 (stuck after a fresh hobby install, plugin server errors) [https://github.com/PostHog/posthog/issues/29706].
   - HN Jul 2026: "Posthog self hosted was basically unusable … hobby deployment script which just pulls the latest build from master which varies from 'somewhat works' to 'completely broken'" [https://news.ycombinator.com/item?id=48847302].
   - PostHog's own reasons for dropping K8s: issues across ingestion, Kafka, ClickHouse, Postgres, and Redis; ClickHouse needs heavy operator effort; Kafka crash-loops on full disks [https://posthog.com/blog/sunsetting-helm-support-posthog].
2. **ClickHouse ops at scale.** Resharding pain was mentioned in an HN comment about PostHog's warehouse woes [https://news.ycombinator.com/item?id=50340452]. PostHog moved the warehouse off ClickHouse to DuckDB, citing no cost-based optimizer, weak lake formats, and version-to-version result changes [https://posthog.com/blog/why-we-rebuilt-our-data-warehouse].
3. **Pricing surprises.**
   - Separately metered products; flag requests (incl. local-eval polls and experiments) spike bills.
   - Reddit anecdotes: flag costs exceeded an entire infra budget; "basically free" vs "$60k/month" [secondary, unverified: https://www.budgetforge.dev/tools/posthog-pricing-2026, https://statsig.com/comparison/alternative-to-posthog-experimentation-statsig].
   - Billing limits exist but are opt-in. Identified events cost up to 4x [docs].
4. **Query slowness, outages, and UI weight.**
   - HN Sep 2025: "simplest queries can take a dozen seconds", plus lost recordings with unclear "not found" [https://news.ycombinator.com/item?id=45217788].
   - HN Nov 2025: "constant outages and slowness" [https://news.ycombinator.com/item?id=45805509].
   - App memory bloat in background tabs [https://github.com/PostHog/posthog/issues/32479].
   - SDK weight: "collecting so much information… making my app unusably slow" [https://news.ycombinator.com/item?id=48848400].
   - Product sprawl: "too many modules and knobs" [https://news.ycombinator.com/item?id=48846293].
5. **Ad-blockers / CSP.** `*.posthog.com` is commonly blocked, so PostHog recommends a reverse proxy (managed proxy offered). Status-0 failures are explicitly logged as possible ad blockers in posthog-js [code: `retry-queue.ts`]. CSP issues: https://github.com/PostHog/posthog/issues/20461. Missing-recordings investigation: https://posthog.com/blog/missing-recordings.
6. **Person-merging gotchas.**
   - Generic IDs (`null`, `"undefined"`, emails used inconsistently) merge unrelated users; hence the illegal-ID list.
   - You can't merge two identified users without `$merge_dangerously`.
   - `$set` is order-of-ingestion, not timestamp.
   - Split IDs doesn't fix past events.
   - Forgetting `reset()` on logout merges people on shared devices.
   - Overflowed hot distinct_ids get no person updates [docs: section 2.2 sources].
7. **Flag latency and consistency.**
   - The client must wait for `/flags` (3s default timeout), so content flickers without bootstrap.
   - Experience continuity is slow and excluded from local eval.
   - Serverless loses the local-eval cache.
   - Local eval can't handle static cohorts or `is_not_set` [docs: local-evaluation].
8. **Replay costs and gaps.**
   - Per-recording billing; retention limited by plan.
   - Mobile replay is wireframes, not pixels, by default.
   - React Native replay was a long-standing request (#13269, 48 comments) [https://github.com/PostHog/posthog/issues/13269].
   - Replay buffering bugs (#40627) [https://github.com/PostHog/posthog/issues/40627].
   - Storing recordings to S3 was a long-time self-host ask (#2142).
9. **Data retention controls.** "Delete events after a certain timeframe" is a top issue (#17031, 99 comments, open since 2023), driven by customers' compliance needs [https://github.com/PostHog/posthog/issues/17031].
10. **Security incidents.**
    - Shai-Hulud 2.0 npm worm (Nov 24, 2025): posthog-js 1.297.3, posthog-node 4.18.1/5.11.3/5.13.3, and posthog-react-native 4.11.1 were trojaned for about 5h; "the largest and most impactful security incident" [https://posthog.com/blog/nov-24-shai-hulud-attack-post-mortem, https://www.theregister.com/2025/11/28/posthog_shaihulud/].
    - The ZDI RCE chain (section 9).
    - Survey-click account takeover (Oct 2026) [https://www.strix.ai/blog/posthog-survey-account-takeover].
11. **Community/support changes.** PostHog closed its Slack community in favor of a forum (Jan 2024, 396 points on HN) [https://news.ycombinator.com/item?id=38987383].

---

## 13. Design implications for a Rust, self-hosted PostHog-class service (opinion)

- **Wire compatibility.** Accept PostHog's capture protocol (`/i/v0/e`, `/batch`, `/e`, `/s`, `/flags?v=2`, `/flags/definitions`, gzip/lz64/base64, `sent_at` skew rule, 23h future clamp, 200-on-drop). This lets us reuse posthog-js and the mobile SDKs, a huge adoption lever. Match the sha1 bucketing exactly so SDK local evaluation agrees.
- **Single binary, embedded storage.**
  - Replace Kafka with an embedded durable log (per-partition segment files) and keep `token:distinct_id` partitioning for person ordering.
  - Replace Postgres persons with an embedded KV (RocksDB/redb/fjall) keyed `(team, distinct_id) → person_id`, with person rows versioned.
  - Columnar event storage: DuckDB or Parquet + DataFusion, or optional ClickHouse for scale.
- **Persons.** Make person profiles opt-in. Store `person_id` on events at write time, plus an overrides table for merges (PoE model) with background squash. Enforce the is_identified guard and the illegal-ID list.
- **Query layer.** Copy the typed query-node schema (Trends/Funnels/Retention/Paths/Stickiness/Lifecycle/Events/Actors/SQL). The SQL layer must inject tenant guards in the planner, not by string concatenation, with allow-listed functions and hard limits (rows, time, memory).
- **Pre-aggregations.** Build a sessions table (UUIDv7 session IDs, argMin/argMax entry and exit, channel tuple, idempotent `uniq` counts) and web pre-aggregates on day one; that is what makes web analytics cheap.
- **Replay.** Ingest rrweb batches into Snappy/zstd blocks appended to object storage (or local files) with byte-range indexes, plus a per-session metadata row with retention TTL.
- **Experiments v1.** Use Bayesian Beta-Binomial / Gamma-Poisson with Monte Carlo or closed-form chance-to-win, a Welch t-test option, and SRM chi-square (p<0.001) from day one.
- **Day-one answers to known pain points.** Per-project event retention/TTL (#17031), tagged releases with migrations and CVEs, first-party proxying (same-origin endpoint) for ad-block resilience, and flag `bootstrap` for SSR.

## Per-event properties in posthog-js (checked 8 November 2026)

`getEventProperties` in `packages/browser-common/src/utils/event-utils.ts` adds these 24 properties to every event: `$browser`, `$browser_language`, `$browser_language_prefix`, `$browser_version`, `$current_url`, `$device`, `$device_type`, `$host`, `$insert_id`, `$lib`, `$lib_version`, `$os`, `$os_version`, `$pathname`, `$raw_user_agent`, `$screen_height`, `$screen_width`, `$time`, `$timezone`, `$timezone_offset`, `$viewport_height`, `$viewport_width`, `$webview_app`, `$webview_app_version`. `calculateEventProperties` in `packages/browser/src/posthog-core.ts` then adds `$session_id`, `$window_id`, `$referrer`, `$referring_domain`, `$groups`, and others, plus registered and persisted properties. This is the repetition that §6's contexts remove.
