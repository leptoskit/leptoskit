//! `POST otlp/v1/metrics`: application-performance metrics, as
//! `leptoskit_apm` exports them (OTLP/HTTP JSON), so route timings land
//! beside the product analytics. Only the app may send them.
//!
//! Delta histograms are stored, one row per window ([`ApmWindow`]); a
//! retried export stores each window once. Anything else is answered as
//! rejected in OTLP's `partialSuccess`, so the exporter knows.
//!
//! [`ApmWindow`]: super::tables::ApmWindow

use std::collections::BTreeMap;

use chrono::DateTime;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use serde_json::Value as Json;
use sha2::Digest as _;
use sha2::Sha256;

use super::ingest::Insert;

/// The metric types stored: OTLP's `AGGREGATION_TEMPORALITY_DELTA`.
const DELTA: i64 = 1;

/// The most windows one request may hold.
pub const MAX_POINTS: usize = 10_000;

/// The most buckets one histogram may have.
const MAX_BUCKETS: usize = 256;

/// Rows per `INSERT`, well inside both engines' parameter limits.
const ROWS_PER_STATEMENT: usize = 200;

/// One window, ready to store.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
	pub id: i64,
	pub metric: String,
	pub unit: String,
	pub service: String,
	pub instance: String,
	pub release: String,
	pub environment: String,
	pub operation: String,
	pub attributes: String,
	pub start: Timestamp,
	pub end: Timestamp,
	pub count: i64,
	pub sum: Option<f64>,
	pub max: Option<f64>,
	pub bucket_counts: String,
	pub bounds: String,
}

/// What a request's body came to.
#[derive(Debug, Default)]
pub struct Parsed {
	pub windows: Vec<Window>,
	/// Data points that weren't stored, and why the first was refused.
	pub rejected: u64,
	pub reason: Option<String>,
}

impl Parsed {
	fn reject(&mut self, count: u64, reason: &str) {
		self.rejected += count;
		self.reason.get_or_insert_with(|| reason.to_owned());
	}
}

/// Reads an OTLP/HTTP JSON `ExportMetricsServiceRequest`.
pub fn parse(body: &Json) -> Parsed {
	let mut parsed = Parsed::default();
	for resource in array(&body["resourceMetrics"]) {
		let resource_attributes = attributes(&resource["resource"]["attributes"]);
		let text = |key: &str| resource_attributes.get(key).cloned().unwrap_or_default();
		let (service, instance) = (text("service.name"), text("service.instance.id"));
		let (release, environment) = (text("service.version"), text("deployment.environment.name"));
		for scope in array(&resource["scopeMetrics"]) {
			for metric in array(&scope["metrics"]) {
				let name = metric["name"].as_str().unwrap_or_default();
				let unit = metric["unit"].as_str().unwrap_or_default();
				let Some(histogram) = metric.get("histogram") else {
					let points = ["sum", "gauge", "exponentialHistogram", "summary"]
						.iter()
						.map(|kind| array(&metric[*kind]["dataPoints"]).len() as u64)
						.sum();
					parsed.reject(points, "only histograms are stored");
					continue;
				};
				let points = array(&histogram["dataPoints"]);
				if integer(&histogram["aggregationTemporality"]) != Some(DELTA) {
					parsed.reject(points.len() as u64, "only delta histograms are stored");
					continue;
				}
				if name.is_empty() || name.len() > 200 || unit.len() > 200 {
					parsed.reject(
						points.len() as u64,
						"a metric name or unit that isn't valid",
					);
					continue;
				}
				for point in points {
					if parsed.windows.len() >= MAX_POINTS {
						parsed.reject(1, "too many data points in one request");
						continue;
					}
					let resource = [&service, &instance, &release, &environment];
					match window(name, unit, resource, point) {
						Ok(window) => parsed.windows.push(window),
						Err(reason) => parsed.reject(1, reason),
					}
				}
			}
		}
	}
	parsed
}

fn window(
	name: &str,
	unit: &str,
	resource: [&String; 4],
	point: &Json,
) -> Result<Window, &'static str> {
	let [service, instance, release, environment] = resource;
	let mut attributes = attributes(&point["attributes"]);
	let operation = attributes.remove("leptoskit.operation").unwrap_or_default();
	let start = integer(&point["startTimeUnixNano"]).ok_or("a data point without a start")?;
	let end = integer(&point["timeUnixNano"]).ok_or("a data point without a time")?;
	let count = integer(&point["count"])
		.filter(|count| *count >= 0)
		.ok_or("a data point without a count")?;
	let bucket_counts: Option<Vec<i64>> =
		array(&point["bucketCounts"]).iter().map(integer).collect();
	let bucket_counts = bucket_counts.ok_or("bucket counts that aren't numbers")?;
	let bounds: Option<Vec<f64>> = array(&point["explicitBounds"])
		.iter()
		.map(Json::as_f64)
		.collect();
	let bounds = bounds.ok_or("bounds that aren't numbers")?;
	if bucket_counts.len() > MAX_BUCKETS
		|| (!bucket_counts.is_empty() && bucket_counts.len() != bounds.len() + 1)
	{
		return Err("bucket counts that don't match the bounds");
	}
	if [service, instance, release, environment]
		.iter()
		.any(|text| text.len() > 200)
		|| operation.len() > 400
	{
		return Err("an attribute that's too long");
	}
	let (Some(start), Some(end)) = (timestamp(start), timestamp(end)) else {
		return Err("a time out of range");
	};
	if end < start {
		return Err("a window that ends before it starts");
	}
	let attributes = serde_json::to_string(&attributes).unwrap_or_default();
	let id = {
		let key = serde_json::to_string(&(
			name,
			service,
			instance,
			&operation,
			&attributes,
			start.timestamp_nanos_opt(),
		))
		.unwrap_or_default();
		let digest = Sha256::digest(key.as_bytes());
		i64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
	};
	Ok(Window {
		id,
		metric: name.into(),
		unit: unit.into(),
		service: service.clone(),
		instance: instance.clone(),
		release: release.clone(),
		environment: environment.clone(),
		operation,
		attributes,
		start,
		end,
		count,
		sum: point["sum"].as_f64(),
		max: point["max"].as_f64(),
		bucket_counts: serde_json::to_string(&bucket_counts).unwrap_or_default(),
		bounds: serde_json::to_string(&bounds).unwrap_or_default(),
	})
}

/// Stores `windows`, each once, in one transaction, and returns how many
/// were new.
pub async fn write(db: &Db, windows: &[Window]) -> leptoskit_db::Result<u64> {
	let tx = db.begin().await?;
	let mut stored = 0;
	for chunk in windows.chunks(ROWS_PER_STATEMENT) {
		let mut insert = Insert::new(
			db.dialect(),
			"apm_windows",
			&[
				"id",
				"metric",
				"unit",
				"service",
				"instance",
				"release",
				"environment",
				"operation",
				"attributes",
				"start",
				"end",
				"count",
				"sum",
				"max",
				"bucket_counts",
				"bounds",
			],
		);
		for window in chunk {
			insert.row(vec![
				Value::Int(window.id),
				Value::Text(window.metric.clone()),
				Value::Text(window.unit.clone()),
				Value::Text(window.service.clone()),
				Value::Text(window.instance.clone()),
				Value::Text(window.release.clone()),
				Value::Text(window.environment.clone()),
				Value::Text(window.operation.clone()),
				Value::Text(window.attributes.clone()),
				Value::Timestamp(window.start),
				Value::Timestamp(window.end),
				Value::Int(window.count),
				window.sum.map_or(Value::Null, Value::Float),
				window.max.map_or(Value::Null, Value::Float),
				Value::Text(window.bucket_counts.clone()),
				Value::Text(window.bounds.clone()),
			]);
		}
		let mut statement = insert.on_conflict_do_nothing("id");
		statement.sql.push_str(" RETURNING \"id\"");
		stored += tx.fetch(statement).await?.len() as u64;
	}
	tx.commit().await?;
	Ok(stored)
}

fn array(value: &Json) -> &[Json] {
	value.as_array().map_or(&[], Vec::as_slice)
}

/// OTLP JSON writes 64-bit integers as strings, and some exporters as
/// numbers: either is read.
fn integer(value: &Json) -> Option<i64> {
	value
		.as_i64()
		.or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn timestamp(nanos: i64) -> Option<Timestamp> {
	(nanos > 0).then(|| DateTime::from_timestamp_nanos(nanos))
}

/// OTLP's `KeyValue` list, as text values by key.
fn attributes(list: &Json) -> BTreeMap<String, String> {
	array(list)
		.iter()
		.filter_map(|pair| {
			let key = pair["key"].as_str()?;
			let value = &pair["value"];
			let text = value["stringValue"]
				.as_str()
				.map(str::to_owned)
				.or_else(|| integer(&value["intValue"]).map(|number| number.to_string()))
				.or_else(|| {
					value["doubleValue"]
						.as_f64()
						.map(|number| number.to_string())
				})
				.or_else(|| value["boolValue"].as_bool().map(|flag| flag.to_string()))?;
			Some((key.to_owned(), text))
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn export(points: &Json) -> Json {
		json!({"resourceMetrics":[{"resource":{"attributes":[
			{"key":"service.name","value":{"stringValue":"shop"}},
			{"key":"service.instance.id","value":{"stringValue":"web-1"}},
			{"key":"service.version","value":{"stringValue":"1.2.0"}},
			{"key":"deployment.environment.name","value":{"stringValue":"production"}}
		]},"scopeMetrics":[{"scope":{"name":"leptoskit_apm"},"metrics":[
			{"name":"leptoskit.operation.duration","unit":"ms","histogram":{"aggregationTemporality":1,"dataPoints":points}}
		]}]}]})
	}

	fn point(start: u64) -> Json {
		json!({"attributes":[
			{"key":"leptoskit.source","value":{"stringValue":"server"}},
			{"key":"leptoskit.operation","value":{"stringValue":"GET /notes/:id"}},
			{"key":"leptoskit.outcome","value":{"stringValue":"ok"}}
		],"startTimeUnixNano":(start * 1_000_000_000).to_string(),"timeUnixNano":((start + 60) * 1_000_000_000).to_string(),
		"count":"7","sum":91.5,"max":30.0,"bucketCounts":["1","5","1"],"explicitBounds":[5.0,25.0]})
	}

	#[test]
	fn a_delta_histogram_becomes_one_window_per_point() {
		let parsed = parse(&export(&json!([
			point(1_762_646_400),
			point(1_762_646_460)
		])));
		assert_eq!(parsed.rejected, 0);
		assert_eq!(parsed.windows.len(), 2);
		let window = &parsed.windows[0];
		assert_eq!(
			(
				window.service.as_str(),
				window.instance.as_str(),
				window.release.as_str(),
				window.environment.as_str()
			),
			("shop", "web-1", "1.2.0", "production")
		);
		assert_eq!(window.operation, "GET /notes/:id");
		assert_eq!(
			window.attributes,
			r#"{"leptoskit.outcome":"ok","leptoskit.source":"server"}"#
		);
		assert_eq!(
			(window.count, window.sum, window.max),
			(7, Some(91.5), Some(30.0))
		);
		assert_eq!(
			(window.bucket_counts.as_str(), window.bounds.as_str()),
			("[1,5,1]", "[5.0,25.0]")
		);
		assert_eq!((window.end - window.start).num_seconds(), 60);
		assert_ne!(parsed.windows[0].id, parsed.windows[1].id);
		assert_eq!(
			parse(&export(&json!([point(1_762_646_400)]))).windows[0].id,
			window.id,
			"the same window, the same id"
		);
	}

	#[test]
	fn what_can_t_be_stored_is_counted_as_rejected() {
		let mut bad = point(1_762_646_400);
		bad["bucketCounts"] = json!(["1"]);
		let mut cumulative = export(&json!([point(1_762_646_400)]));
		cumulative["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["histogram"]
			["aggregationTemporality"] = json!(2);
		let parsed = parse(&export(&json!([bad, point(1_762_646_400)])));
		assert_eq!((parsed.windows.len(), parsed.rejected), (1, 1));
		assert_eq!(
			parsed.reason.as_deref(),
			Some("bucket counts that don't match the bounds")
		);
		let parsed = parse(&cumulative);
		assert_eq!((parsed.windows.len(), parsed.rejected), (0, 1));
		let gauge = json!({"resourceMetrics":[{"scopeMetrics":[{"metrics":[{"name":"g","gauge":{"dataPoints":[{}, {}]}}]}]}]});
		assert_eq!(parse(&gauge).rejected, 2);
	}
}
