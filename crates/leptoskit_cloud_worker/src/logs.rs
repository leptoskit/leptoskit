//! A job's log, on its way to the control plane.
//!
//! Whatever runs the job writes lines to a [`LogSink`]. A task sends them
//! in order, in chunks within the protocol's bounds. The log is for the
//! person watching the build: when the control plane can't be reached the
//! job goes on, and lines past what is kept waiting are dropped and
//! counted.

use std::time::Duration;

use leptoskit_cloud::workers::MAX_LOG_LINE_BYTES;
use leptoskit_cloud::workers::MAX_LOG_LINES;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::client::CallError;
use crate::client::Control;

/// The most lines kept waiting to be sent.
const WAITING: usize = 20_000;

/// The most bytes of lines in one chunk: under the protocol's 64 KiB, with
/// room for how the lines are written on the wire.
const CHUNK_BYTES: usize = 32 * 1024;

/// How long lines are gathered before a chunk that isn't full is sent.
///
/// Batches log writes to keep S3 PUT and control-plane database transaction
/// overhead low; full chunks (`CHUNK_BYTES` or `MAX_LOG_LINES`) still flush
/// immediately.
const GATHER: Duration = Duration::from_secs(2);

/// How many times one chunk is sent before the log is given up on.
const TRIES: u32 = 6;

/// Where a job's log lines go.
#[derive(Clone, Debug)]
pub struct LogSink {
	lines: mpsc::Sender<String>,
	dropped: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl LogSink {
	/// Adds a line. Never waits: a line there is no room for is dropped.
	pub fn line(&self, text: impl Into<String>) {
		if self.lines.try_send(cut(text.into())).is_err() {
			self.dropped
				.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		}
	}

	/// A sink and the lines written to it, for tests of what writes a log.
	#[must_use]
	pub fn collecting() -> (Self, mpsc::Receiver<String>) {
		let (lines, receiver) = mpsc::channel(WAITING);
		(
			Self {
				lines,
				dropped: std::sync::Arc::default(),
			},
			receiver,
		)
	}
}

/// `line` as the protocol takes one: a single line, 4 KiB at most, with
/// no control character but a tab, so a build's output can't move a
/// terminal's cursor or colour what a person reads after it.
fn cut(mut line: String) -> String {
	if line.contains(|c: char| c.is_control() && c != '\t') {
		line = line
			.chars()
			.map(|c| if c.is_control() && c != '\t' { ' ' } else { c })
			.collect();
	}

	if line.len() > MAX_LOG_LINE_BYTES {
		let mut end = MAX_LOG_LINE_BYTES;

		while !line.is_char_boundary(end) {
			end -= 1;
		}

		line.truncate(end);
	}

	line
}

/// Takes the next chunk from `waiting`: as many lines as fit the bounds,
/// and one at least.
fn chunk(waiting: &mut std::collections::VecDeque<String>) -> Vec<String> {
	let mut lines = Vec::new();
	let mut bytes = 0;

	while let Some(next) = waiting.front() {
		if !lines.is_empty() && (lines.len() >= MAX_LOG_LINES || bytes + next.len() > CHUNK_BYTES) {
			break;
		}

		bytes += next.len();
		lines.extend(waiting.pop_front());
	}

	lines
}

/// Sends one chunk, again while the control plane can't be reached.
/// Sending the same chunk twice is accepted, so a lost answer costs
/// nothing.
async fn send(
	control: &Control,
	job: Uuid,
	attempt: u32,
	seq: u32,
	lines: &[String],
) -> Result<(), CallError> {
	let mut wait = Duration::from_millis(500);
	let mut tries = 0;

	loop {
		tries += 1;

		match control.append_log(job, attempt, seq, lines.to_vec()).await {
			Ok(()) => return Ok(()),
			Err(error) if error.is_transient() && tries < TRIES => {
				tokio::time::sleep(wait).await;
				wait = (wait * 2).min(Duration::from_secs(8));
			}
			Err(error) => return Err(error),
		}
	}
}

/// Starts sending the log of `attempt` of `job`. The task ends once every
/// [`LogSink`] is dropped and what they wrote is sent, or when the control
/// plane no longer takes the log; it answers how many lines were never
/// sent.
pub(crate) fn ship(
	control: Control,
	job: Uuid,
	attempt: u32,
) -> (LogSink, tokio::task::JoinHandle<u64>) {
	let (lines, mut receiver) = mpsc::channel::<String>(WAITING);
	let dropped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
	let sink = LogSink {
		lines,
		dropped: std::sync::Arc::clone(&dropped),
	};
	let task = tokio::spawn(async move {
		let mut waiting = std::collections::VecDeque::new();
		let mut seq = 0;
		let mut open = true;
		let mut unsent = 0_u64;

		while open || !waiting.is_empty() {
			if waiting.is_empty() {
				match receiver.recv().await {
					Some(line) => waiting.push_back(line),
					None => break,
				}
			}

			// Gather what else arrives soon, so a chatty build is a few
			// chunks per second and not one call a line, while flushing
			// immediately once a chunk reaches CHUNK_BYTES or MAX_LOG_LINES.
			let gathered = tokio::time::sleep(GATHER);
			tokio::pin!(gathered);
			let mut waiting_bytes: usize = waiting.iter().map(String::len).sum();

			while open && waiting.len() < MAX_LOG_LINES && waiting_bytes < CHUNK_BYTES {
				tokio::select! {
					line = receiver.recv() => match line {
						Some(line) => {
							waiting_bytes += line.len();
							waiting.push_back(line);
						}
						None => open = false,
					},
					() = &mut gathered => break,
				}
			}

			let lines = chunk(&mut waiting);

			if send(&control, job, attempt, seq, &lines).await.is_err() {
				// The lease is gone, or the log is out of step: nothing
				// more of it will be taken.
				unsent += lines.len() as u64 + waiting.len() as u64;
				receiver.close();

				while receiver.recv().await.is_some() {
					unsent += 1;
				}

				break;
			}

			seq += 1;
		}

		unsent + dropped.load(std::sync::atomic::Ordering::Relaxed)
	});
	(sink, task)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_line_holds_no_control_characters_but_tabs() {
		assert_eq!(cut("a\tb\u{1b}[2Jc\rd\ne".to_owned()), "a\tb [2Jc d e");
	}

	#[test]
	fn a_line_is_one_line_of_four_kibibytes_at_most() {
		assert_eq!(cut("one\ntwo\r\n".to_owned()), "one two  ");
		let long = "é".repeat(MAX_LOG_LINE_BYTES);
		let cut = cut(long);
		assert!(cut.len() <= MAX_LOG_LINE_BYTES);
		assert!(cut.chars().all(|character| character == 'é'));
	}

	#[test]
	fn a_chunk_stays_within_the_bounds() {
		let mut waiting: std::collections::VecDeque<String> =
			(0..2_500).map(|number| number.to_string()).collect();
		let first = chunk(&mut waiting);
		assert_eq!(first.len(), MAX_LOG_LINES);
		assert_eq!(first[0], "0");
		assert_eq!(waiting.len(), 1_500);

		// Long lines fill a chunk by their bytes, in order.
		let mut waiting: std::collections::VecDeque<String> =
			(0..20).map(|_| "x".repeat(MAX_LOG_LINE_BYTES)).collect();
		let first = chunk(&mut waiting);
		assert_eq!(first.len(), CHUNK_BYTES / MAX_LOG_LINE_BYTES);
		assert_eq!(waiting.len(), 20 - first.len());

		// One line always goes, and nothing is nothing.
		let mut waiting = std::collections::VecDeque::from(["only".to_owned()]);
		assert_eq!(chunk(&mut waiting), ["only"]);
		assert_eq!(chunk(&mut waiting), Vec::<String>::new());
	}

	#[tokio::test]
	async fn a_sink_never_waits() {
		let (sink, mut lines) = LogSink::collecting();

		for number in 0..WAITING + 5 {
			sink.line(number.to_string());
		}

		assert_eq!(sink.dropped.load(std::sync::atomic::Ordering::Relaxed), 5);
		assert_eq!(lines.recv().await.as_deref(), Some("0"));
	}
}
