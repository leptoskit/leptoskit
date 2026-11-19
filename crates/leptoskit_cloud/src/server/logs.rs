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
//! Every line is redacted before it is stored ([`redact_lines`]): worker keys,
//! API tokens, pairing codes, presigned addresses' signatures, bearer
//! tokens, the password in an address such as `postgres://user:…@host`, and
//! the values the control plane was configured with as secrets. An
//! environment's own secrets are not known here: one an app prints in
//! another shape is kept as printed. A chunk expires with the log's retention,
//! and the sweeper deletes its row and its object.

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
/// they appear. Control characters are written out ([`plain`]).
#[cfg(test)]
pub(crate) fn redact(line: &str, secrets: &[String]) -> String {
	redact_lines(&[line], secrets).remove(0)
}

/// `lines`, in order, each with every secret it carries replaced, and with
/// its control characters written out ([`plain`]).
///
/// Secrets are looked for in the lines run together as well as in each
/// alone, so one split over two or more lines of a chunk is replaced in
/// every line it touches. Nothing is remembered from chunk to chunk; at a
/// chunk's edges, a first line that starts with the end of a configured
/// secret, or a last line that ends with the start of one, loses that part
/// when it is [`MIN_SECRET`] bytes or more. A worker key, token, or
/// signature split over two chunks is hidden only in the chunk its marker
/// is in.
pub(crate) fn redact_lines(lines: &[&str], secrets: &[String]) -> Vec<String> {
	let mut spans: Vec<Vec<(usize, usize)>> = lines
		.iter()
		.map(|line| secret_spans(line, secrets))
		.collect();
	let joined: String = lines.concat();
	let mut offset = 0;
	let bounds: Vec<(usize, usize)> = lines
		.iter()
		.map(|line| {
			let start = offset;
			offset += line.len();
			(start, offset)
		})
		.collect();

	// Each span of the joined text goes to the lines it touches. Spans are
	// merged first, so they and the lines are walked once, in step.
	let mut line = 0;

	for (start, end) in merged(secret_spans(&joined, secrets)) {
		while line < bounds.len() && bounds[line].1 <= start {
			line += 1;
		}

		let mut touched = line;

		while touched < bounds.len() && bounds[touched].0 < end {
			let (from, to) = bounds[touched];
			let (start, end) = (start.max(from), end.min(to));

			if start < end {
				spans[touched].push((start - from, end - from));
			}

			touched += 1;
		}
	}

	if let (Some(first), Some(first_spans)) = (lines.first(), spans.first_mut())
		&& let Some(length) = edge(first, secrets, Edge::Start)
	{
		first_spans.push((0, length));
	}

	if let (Some(last), Some(last_spans)) = (lines.last(), spans.last_mut())
		&& let Some(length) = edge(last, secrets, Edge::End)
	{
		last_spans.push((last.len() - length, last.len()));
	}

	lines
		.iter()
		.zip(spans)
		.map(|(line, spans)| plain(&replace(line, spans)))
		.collect()
}

/// The shortest configured secret, or part of one at a chunk's edge, that
/// is hidden: shorter text matches too much that isn't secret.
const MIN_SECRET: usize = 8;

/// Which end of a line [`edge`] looks at.
#[derive(Clone, Copy)]
enum Edge {
	Start,
	End,
}

/// How many bytes at `line`'s `edge` are the cut-off part of a configured
/// secret: the longest end of one the line starts with, or start of one it
/// ends with, of [`MIN_SECRET`] bytes or more.
fn edge(line: &str, secrets: &[String], edge: Edge) -> Option<usize> {
	secrets
		.iter()
		.filter(|secret| secret.len() > MIN_SECRET)
		.filter_map(|secret| {
			(MIN_SECRET..secret.len())
				.rev()
				.filter(|&length| {
					match edge {
						Edge::Start => secret.is_char_boundary(secret.len() - length),
						Edge::End => secret.is_char_boundary(length),
					}
				})
				.find(|&length| {
					match edge {
						Edge::Start => line.starts_with(&secret[secret.len() - length..]),
						Edge::End => line.ends_with(&secret[..length]),
					}
				})
		})
		.max()
}

/// `spans` in order, with those that touch or overlap made one.
fn merged(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
	spans.sort_unstable();
	let mut out: Vec<(usize, usize)> = Vec::with_capacity(spans.len());

	for (start, end) in spans {
		match out.last_mut() {
			Some(last) if start <= last.1 => last.1 = last.1.max(end),
			_ => out.push((start, end)),
		}
	}

	out
}

/// `line` with each of `spans` replaced by [`REDACTED`]; spans that touch
/// or overlap are replaced once.
fn replace(line: &str, spans: Vec<(usize, usize)>) -> String {
	if spans.is_empty() {
		return line.to_owned();
	}

	let mut out = String::with_capacity(line.len());
	let mut at = 0;

	for (start, end) in merged(spans) {
		out.push_str(&line[at..start]);
		out.push_str(REDACTED);
		at = end;
	}

	out.push_str(&line[at..]);
	out
}

/// `text` with every control character but a tab written out as
/// `\u{…}`, so a log's reader's terminal is never told to move its cursor,
/// change its title, or hide text.
pub(crate) fn plain(text: &str) -> String {
	if !text.chars().any(|c| c.is_control() && c != '\t') {
		return text.to_owned();
	}

	let mut out = String::with_capacity(text.len());

	for c in text.chars() {
		if c.is_control() && c != '\t' {
			out.extend(c.escape_unicode());
		} else {
			out.push(c);
		}
	}

	out
}

/// Where the secrets in `text` are, as byte ranges: see the
/// [module docs](self).
fn secret_spans(text: &str, secrets: &[String]) -> Vec<(usize, usize)> {
	let mut spans = Vec::new();

	for secret in secrets.iter().filter(|secret| secret.len() >= MIN_SECRET) {
		spans.extend(
			text.match_indices(secret.as_str())
				.map(|(start, found)| (start, start + found.len())),
		);
	}

	// Credentials: the lookup part stays, so a reader can tell which.
	for prefix in ["lkc_", "lkw_"] {
		let mut from = 0;

		// Each search starts past what the last one read, so the text is
		// read once however many prefixes it holds.
		while let Some(found) = text[from..].find(prefix) {
			let rest_at = from + found + prefix.len();
			let rest = &text[rest_at..];
			let lookup = hex_run(rest);
			from = rest_at + lookup;

			if lookup >= 12 && rest[lookup..].starts_with('_') {
				let start = from + 1;
				from = start + hex_run(&rest[lookup + 1..]);
				spans.push((start, from));
			}
		}
	}

	after(&mut spans, text, "lkp_", hex_run);

	for parameter in [
		"X-Amz-Signature=",
		"X-Amz-Credential=",
		"X-Amz-Security-Token=",
		"x-amz-signature=",
		"x-amz-credential=",
		"x-amz-security-token=",
		"sig=",
	] {
		after(&mut spans, text, parameter, |rest| {
			rest.find(['&', ' ', '"', '\'', '\n', '\t'])
				.unwrap_or(rest.len())
		});
	}

	for scheme in ["Bearer ", "bearer "] {
		after(&mut spans, text, scheme, |rest| {
			rest.find([' ', '"', '\'', '\n', '\t', ',', ';'])
				.unwrap_or(rest.len())
		});
	}

	// A password in an address, `scheme://user:password@host`, as a
	// database's is: a server that can't connect prints the whole of it.
	let mut from = 0;

	while let Some(found) = text[from..].find("://") {
		let authority_at = from + found + 3;
		let rest = &text[authority_at..];
		let end = rest
			.find(['/', '?', '#', ' ', '"', '\'', '\n', '\t'])
			.unwrap_or(rest.len());
		let authority = &rest[..end];

		if let Some(at) = authority.rfind('@')
			&& let Some(colon) = authority[..at].find(':')
		{
			spans.push((authority_at + colon + 1, authority_at + at));
		}

		from = authority_at + end;
	}

	spans.retain(|(start, end)| start < end);
	spans
}

/// `text` with every secret it can be seen to carry replaced: for a reason
/// a worker gives, which is shown on a deployment's sheet.
pub(crate) fn scrub(text: &str) -> String {
	redact_lines(&[text], &[]).remove(0)
}

/// The length of the run of hex digits `text` starts with.
fn hex_run(text: &str) -> usize {
	text.bytes().take_while(u8::is_ascii_hexdigit).count()
}

/// Adds what `span` measures after each `marker` in `text` to `spans`. A
/// marker inside what the one before it measured isn't looked at again, so
/// the text is read once however many markers it holds.
fn after(spans: &mut Vec<(usize, usize)>, text: &str, marker: &str, span: impl Fn(&str) -> usize) {
	let mut from = 0;

	while let Some(found) = text[from..].find(marker) {
		let start = from + found + marker.len();
		from = start + span(&text[start..]);
		spans.push((start, from));
	}
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
	let deployment_id: Id<Deployment> = deployment.id;
	build_page(cx, after, limit, |chunk| {
		chunk.deployment_id.eq(Some(deployment_id))
	})
	.await
}

/// A page of the build-kind log whose chunks `which` picks: a deployment's
/// build, or one publish job's run.
pub(crate) async fn build_page(
	cx: &rpc::QueryCx,
	after: Option<u64>,
	limit: Option<u32>,
	which: impl Fn(&<LogChunk as leptoskit_db::Table>::Columns) -> leptoskit_db::Expr,
) -> rpc::Result<LogPage> {
	let services = services(cx.context())?;
	let after = after.unwrap_or_default();
	let limit = page_size(limit);
	let after_line = i64::try_from(after).unwrap_or(i64::MAX);
	let chunks = LogChunk::query()
		.filter(|chunk| {
			which(chunk)
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
		assert_eq!(
			redact(
				"error connecting to postgres://env_ab:c0ffee@pg.svc:5432/env_ab?sslmode=require",
				&[]
			),
			"error connecting to postgres://env_ab:[redacted]@pg.svc:5432/env_ab?sslmode=require"
		);
		assert_eq!(
			redact("see https://example.com/a:b@c and http://host:80/", &[]),
			"see https://example.com/a:b@c and http://host:80/"
		);
		assert_eq!(scrub("redis://:hunter2@cache"), "redis://:[redacted]@cache");
		assert_eq!(redact("lkw_short", &[]), "lkw_short");
	}

	#[test]
	fn a_secret_split_over_lines_is_hidden_in_each() {
		let secrets = ["s3cr3t-value-of-some-length".to_owned()];
		let key = format!("lkw_0123456789ab_{}", "c0ffee".repeat(10));
		let (key_start, key_end) = key.split_at(40);
		let lines = [
			"the secret is s3cr3t-val",
			"ue-of-some-length, and the key is",
			key_start,
			key_end,
			"nothing here",
		];
		let redacted = redact_lines(&lines, &secrets);
		assert_eq!(redacted[0], "the secret is [redacted]");
		assert_eq!(redacted[1], "[redacted], and the key is");
		assert_eq!(redacted[2], "lkw_0123456789ab_[redacted]");
		assert_eq!(redacted[3], "[redacted]");
		assert_eq!(redacted[4], "nothing here");
	}

	#[test]
	fn a_configured_secret_cut_by_a_chunks_edge_is_hidden() {
		let secrets = ["s3cr3t-value-of-some-length".to_owned()];
		// The chunk before ended with the secret's start, and this one
		// starts with its end.
		assert_eq!(
			redact_lines(&["key: s3cr3t-value-of"], &secrets),
			["key: [redacted]"]
		);
		assert_eq!(
			redact_lines(&["-some-length is the key"], &secrets),
			["[redacted] is the key"]
		);
		// Seven bytes of it could be anything.
		assert_eq!(redact_lines(&["key: s3cr3t-"], &secrets), ["key: s3cr3t-"]);
	}

	#[test]
	fn text_full_of_markers_is_read_once() {
		// 64 KiB of `sig=` in a thousand lines with no delimiter: each
		// marker's span used to run to the end of the text, and reach every
		// line after it.
		let line = "sig=".repeat(16);
		let lines: Vec<&str> = std::iter::repeat_n(line.as_str(), 1000).collect();
		let started = std::time::Instant::now();
		let redacted = redact_lines(&lines, &[]);
		assert!(
			started.elapsed() < std::time::Duration::from_secs(2),
			"{:?}",
			started.elapsed()
		);
		assert_eq!(redacted.len(), 1000);
		assert_eq!(redacted[0], "sig=[redacted]");
		assert_eq!(redacted[999], "[redacted]");
		assert_eq!(spans_of(&"lkw_".repeat(16_000)), 0);
	}

	/// How many spans `secret_spans` finds in `text`.
	fn spans_of(text: &str) -> usize {
		secret_spans(text, &[]).len()
	}

	#[test]
	fn control_characters_are_written_out() {
		assert_eq!(
			plain("\u{1b}[2Jgone\r\u{7}\ttab \u{9b}31m"),
			"\\u{1b}[2Jgone\\u{d}\\u{7}\ttab \\u{9b}31m"
		);
		assert_eq!(plain("plain text, ünïcödé"), "plain text, ünïcödé");
		assert_eq!(redact("a\u{1b}b", &[]), "a\\u{1b}b");
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
