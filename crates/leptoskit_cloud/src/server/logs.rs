//! Logs: a build's, which its worker appends while it runs, and an
//! environment's processes', which its cluster worker reports.
//!
//! Lines are kept in the object store in chunks, one object each; the
//! database keeps an index row per chunk (`log_chunks`) with its place in
//! the log, and the reader pages through the index and reads the objects
//! it needs. A chunk's row is written before its object, so a reader may
//! find a row whose object isn't there yet, or failed to arrive: it skips
//! it, and the worker that sent it sends it again.
//!
//! Every line is redacted before it is stored ([`redact`]): worker keys,
//! API tokens, pairing codes, presigned addresses' signatures, bearer
//! tokens, and the values the control plane was configured with as
//! secrets. A chunk expires with the log's retention, and the sweeper
//! deletes its row and its object.

use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;

use super::authorize;
use super::projects::environment;
use super::services;
use super::storage::Bucket;
use crate::access::Permission;
use crate::deployments::LogEntry;
use crate::deployments::LogPage;
use crate::deployments::MAX_LOG_PAGE;
use crate::tables::Deployment;
use crate::tables::LogChunk;
use crate::tables::LogKind;
use crate::workers::MAX_LOG_LINE_BYTES;
use crate::workers::MAX_LOG_LINES;
use crate::workers::MAX_STATUS_LOG_LINES;

/// How many lines a page has unless the caller asks for fewer or more.
const DEFAULT_PAGE: u32 = 200;

/// The most text one page carries, so a live page stays well under the
/// live socket's limit.
const MAX_PAGE_BYTES: usize = 256 * 1024;

/// The most chunks one page reads.
const MAX_PAGE_CHUNKS: u64 = 200;

/// How far back `runtime_logs` looks unless asked.
const DEFAULT_SINCE_MINUTES: u32 = 60;

/// What a redacted secret becomes.
const REDACTED: &str = "[redacted]";

/// One line, as a chunk's object keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredLine {
	pub(crate) at: Timestamp,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(crate) process: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(crate) pod: Option<String>,
	pub(crate) text: String,
}

/// SHA-256 of `lines` as a worker sent them, so a chunk sent again is
/// recognised.
pub(crate) fn chunk_digest<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
	let mut hasher = sha2::Sha256::new();

	for line in lines {
		hasher.update(u64::try_from(line.len()).unwrap_or(u64::MAX).to_be_bytes());
		hasher.update(line.as_bytes());
	}

	hex::encode(hasher.finalize())
}

/// `line` cut to [`MAX_LOG_LINE_BYTES`] at a character boundary.
pub(crate) fn cut(line: &str) -> String {
	if line.len() <= MAX_LOG_LINE_BYTES {
		return line.to_owned();
	}

	let mut end = MAX_LOG_LINE_BYTES;

	while !line.is_char_boundary(end) {
		end -= 1;
	}

	format!("{}…", &line[..end])
}

/// The line with every secret it carries replaced: see the
/// [module docs](self). `secrets` are configured values to hide wherever
/// they appear.
pub(crate) fn redact(line: &str, secrets: &[String]) -> String {
	let mut out = line.to_owned();

	for secret in secrets.iter().filter(|secret| secret.len() >= 8) {
		if out.contains(secret.as_str()) {
			out = out.replace(secret.as_str(), REDACTED);
		}
	}

	// Credentials: the lookup part stays, so a reader can tell which.
	for prefix in ["lkc_", "lkw_"] {
		out = redact_credentials(&out, prefix);
	}

	out = redact_after(&out, "lkp_", hex_run);

	for parameter in [
		"X-Amz-Signature=",
		"X-Amz-Credential=",
		"X-Amz-Security-Token=",
		"x-amz-signature=",
		"x-amz-credential=",
		"x-amz-security-token=",
		"sig=",
	] {
		out = redact_after(&out, parameter, |rest| {
			rest.find(['&', ' ', '"', '\'', '\n', '\t'])
				.unwrap_or(rest.len())
		});
	}

	for scheme in ["Bearer ", "bearer "] {
		out = redact_after(&out, scheme, |rest| {
			rest.find([' ', '"', '\'', '\n', '\t', ',', ';'])
				.unwrap_or(rest.len())
		});
	}

	out
}

/// The length of the run of hex digits `text` starts with.
fn hex_run(text: &str) -> usize {
	text.bytes().take_while(u8::is_ascii_hexdigit).count()
}

/// Replaces what `span` measures after each `marker` with
/// [`REDACTED`], when it is anything.
fn redact_after(text: &str, marker: &str, span: impl Fn(&str) -> usize) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;

	while let Some(index) = rest.find(marker) {
		let after = index + marker.len();
		out.push_str(&rest[..after]);
		rest = &rest[after..];
		let length = span(rest);

		if length > 0 && !rest.starts_with(REDACTED) {
			out.push_str(REDACTED);
			rest = &rest[length..];
		}
	}

	out.push_str(rest);
	out
}

/// `<prefix><12 hex>_<secret>` keeps its prefix and lookup part, and loses
/// its secret.
fn redact_credentials(text: &str, prefix: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;

	while let Some(index) = rest.find(prefix) {
		let start = index + prefix.len();
		out.push_str(&rest[..start]);
		rest = &rest[start..];
		let lookup = hex_run(rest);

		if lookup >= 12 && rest[lookup..].starts_with('_') {
			let secret = hex_run(&rest[lookup + 1..]);
			out.push_str(&rest[..=lookup]);
			out.push_str(REDACTED);
			rest = &rest[lookup + 1 + secret..];
		}
	}

	out.push_str(rest);
	out
}

/// Reads one chunk's lines; a chunk whose object is missing has none.
async fn chunk_lines(
	store: &super::storage::ObjectStore,
	chunk: &LogChunk,
) -> rpc::Result<Vec<StoredLine>> {
	let Some(bytes) = store.get(Bucket::Logs, &chunk.object_key).await? else {
		return Ok(Vec::new());
	};
	serde_json::from_slice(&bytes)
		.map_err(|_| rpc::Error::server("a log chunk in the store is malformed"))
}

/// A page's size: `limit`, or the default, at most [`MAX_LOG_PAGE`].
fn page_size(limit: Option<u32>) -> usize {
	usize::try_from(limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_LOG_PAGE)).unwrap_or(1)
}

/// The most lines a chunk holds, as a line count: chunks that end after a
/// line start no earlier than this many lines before it.
fn longest(lines: usize) -> i64 {
	i64::try_from(lines).unwrap_or(i64::MAX)
}

fn days(duration: std::time::Duration) -> u32 {
	u32::try_from(duration.as_secs() / 86_400).unwrap_or(u32::MAX)
}

/// Adds lines to `page` until it is full, numbering them from the chunk's
/// first line; says whether it filled.
fn fill(
	page: &mut Vec<LogEntry>,
	bytes: &mut usize,
	limit: usize,
	chunk: &LogChunk,
	lines: Vec<StoredLine>,
	after: u64,
	keep: &impl Fn(&StoredLine) -> bool,
) -> bool {
	let first = u64::try_from(chunk.first_line).unwrap_or_default();

	for (offset, line) in (0_u64..).zip(lines) {
		let n = first + offset;

		if n <= after || !keep(&line) {
			continue;
		}

		if page.len() >= limit || *bytes + line.text.len() > MAX_PAGE_BYTES {
			return true;
		}

		*bytes += line.text.len();
		page.push(LogEntry {
			n,
			at: line.at,
			attempt: (chunk.kind == LogKind::Build)
				.then(|| u32::try_from(chunk.attempt).unwrap_or_default()),
			process: line.process,
			pod: line.pod,
			text: line.text,
		});
	}

	false
}

/// `build_logs`'s body.
pub(crate) async fn build_logs(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	number: u64,
	after: Option<u64>,
	limit: Option<u32>,
) -> rpc::Result<LogPage> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let deployment = super::deployments::by_number(cx.db(), &environment, number).await?;
	let services = services(cx.context())?;
	let after = after.unwrap_or_default();
	let limit = page_size(limit);
	let deployment_id: Id<Deployment> = deployment.id;
	let after_line = i64::try_from(after).unwrap_or(i64::MAX);
	let chunks = LogChunk::query()
		.filter(|chunk| {
			chunk.deployment_id.eq(Some(deployment_id))
				& chunk.kind.eq(LogKind::Build)
				& chunk
					.first_line
					.gt(after_line.saturating_sub(longest(MAX_LOG_LINES)))
		})
		.order_by(|chunk| chunk.first_line.asc())
		.limit(MAX_PAGE_CHUNKS)
		.all(cx.db())
		.await?;
	let mut lines = Vec::new();
	let mut bytes = 0;
	let mut full = false;

	for chunk in chunks
		.iter()
		.filter(|chunk| u64::try_from(chunk.first_line + chunk.lines).unwrap_or_default() > after)
	{
		let stored = chunk_lines(&services.shipping().store, chunk).await?;

		if fill(&mut lines, &mut bytes, limit, chunk, stored, after, &|_| {
			true
		}) {
			full = true;
			break;
		}
	}

	Ok(LogPage {
		next: full.then(|| lines.last().map(|line| line.n)).flatten(),
		lines,
		retention_days: days(services.shipping().settings.build_logs),
	})
}

/// `runtime_logs`'s arguments.
pub(crate) struct RuntimeLogQuery {
	pub(crate) organization: String,
	pub(crate) project: String,
	pub(crate) environment: String,
	pub(crate) process: Option<String>,
	pub(crate) since_minutes: Option<u32>,
	pub(crate) contains: Option<String>,
	pub(crate) after: Option<u64>,
	pub(crate) limit: Option<u32>,
}

/// `runtime_logs`'s body.
pub(crate) async fn runtime_logs(
	cx: &rpc::QueryCx,
	query: RuntimeLogQuery,
) -> rpc::Result<LogPage> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		&query.organization,
		&query.project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, &query.environment).await?;
	let services = services(cx.context())?;
	let retention = services.shipping().settings.runtime_logs;
	let retention_minutes = u32::try_from(retention.as_secs() / 60).unwrap_or(u32::MAX);
	let since = query
		.since_minutes
		.unwrap_or(DEFAULT_SINCE_MINUTES)
		.clamp(1, retention_minutes.max(1));
	let from = leptoskit_db::now() - chrono::TimeDelta::minutes(i64::from(since));
	let after = query.after.unwrap_or_default();
	let after_line = i64::try_from(after).unwrap_or(i64::MAX);
	let limit = page_size(query.limit);
	let environment_id = environment.id;

	if let Some(contains) = &query.contains
		&& contains.len() > MAX_LOG_LINE_BYTES
	{
		return Err(rpc::Error::invalid(format!(
			"invalid_filter: `contains` is at most {MAX_LOG_LINE_BYTES} bytes"
		)));
	}

	let chunks = LogChunk::query()
		.filter(|chunk| {
			chunk.environment_id.eq(environment_id)
				& chunk.kind.eq(LogKind::Runtime)
				& chunk.created_at.gte(from)
				& chunk
					.first_line
					.gt(after_line.saturating_sub(longest(MAX_STATUS_LOG_LINES)))
		})
		.order_by(|chunk| chunk.first_line.asc())
		.limit(MAX_PAGE_CHUNKS)
		.all(cx.db())
		.await?;
	let keep = |line: &StoredLine| {
		query
			.process
			.as_ref()
			.is_none_or(|process| line.process.as_ref() == Some(process))
			&& query
				.contains
				.as_ref()
				.is_none_or(|needle| line.text.contains(needle.as_str()))
	};
	let mut lines = Vec::new();
	let mut bytes = 0;
	let mut full = false;

	for chunk in chunks
		.iter()
		.filter(|chunk| u64::try_from(chunk.first_line + chunk.lines).unwrap_or_default() > after)
	{
		let stored = chunk_lines(&services.shipping().store, chunk).await?;

		if fill(&mut lines, &mut bytes, limit, chunk, stored, after, &keep) {
			full = true;
			break;
		}
	}

	Ok(LogPage {
		next: full.then(|| lines.last().map(|line| line.n)).flatten(),
		lines,
		retention_days: days(retention),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn secrets_are_redacted_and_lookups_kept() {
		let worker = format!("lkw_0123456789ab_{}", "f".repeat(64));
		let token = format!("lkc_abcdefabcdef_{}", "0".repeat(64));
		let code = format!("lkp_{}", "a".repeat(32));
		let line = format!(
			"key {worker} token={token} code {code} url https://s/x?X-Amz-Credential=AK%2F1&\
			 X-Amz-Signature=deadbeef&sig=cafe Authorization: Bearer abc.def s3cr3t-value!"
		);
		let redacted = redact(&line, &["s3cr3t-value".to_owned()]);

		for secret in [
			&"f".repeat(64),
			&"0".repeat(64),
			&"a".repeat(32),
			"deadbeef",
			"AK%2F1",
			"cafe",
			"abc.def",
			"s3cr3t-value",
		] {
			assert!(!redacted.contains(secret), "{secret} in {redacted}");
		}

		assert!(
			redacted.contains("lkw_0123456789ab_[redacted]"),
			"{redacted}"
		);
		assert!(
			redacted.contains("lkc_abcdefabcdef_[redacted]"),
			"{redacted}"
		);
		assert_eq!(redact("nothing to see", &[]), "nothing to see");
		assert_eq!(redact("lkw_short", &[]), "lkw_short");
	}

	#[test]
	fn long_lines_are_cut_on_a_character_boundary() {
		let line = "é".repeat(MAX_LOG_LINE_BYTES);
		let cut = cut(&line);
		assert!(cut.len() <= MAX_LOG_LINE_BYTES + '…'.len_utf8());
		assert!(cut.ends_with('…'));
		assert_eq!(super::cut("short"), "short");
	}

	#[test]
	fn a_chunks_digest_depends_on_its_lines_and_their_boundaries() {
		assert_eq!(chunk_digest(["a", "b"]), chunk_digest(["a", "b"]));
		assert_ne!(chunk_digest(["a", "b"]), chunk_digest(["ab"]));
		assert_ne!(chunk_digest(["a", "b"]), chunk_digest(["b", "a"]));
	}
}
