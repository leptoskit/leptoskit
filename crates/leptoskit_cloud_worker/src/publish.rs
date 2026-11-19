//! A publisher that is a program the operator names: `lk publish`, or a
//! script that runs it inside a sandbox.
//!
//! The program is run once for each publish, in the unpacked source, with
//! a cleared environment and these variables:
//!
//! - `LEPTOSKIT_PUBLISH_SOURCE`: the source directory, also where it starts;
//! - `LEPTOSKIT_PUBLISH_INPUT`: a JSON file holding a [`PublishRequest`];
//! - `LEPTOSKIT_PUBLISH_STORES`: the stores, separated by spaces, in `lk
//!   publish`'s order;
//! - `LEPTOSKIT_PUBLISH_SUBMIT`: `1` when each upload is also sent for review
//!   or release, and `0` when it isn't;
//! - `LEPTOSKIT_PUBLISH_OUTPUT`: where it writes `lk publish --json`'s report;
//! - each store credential the job carries, by the name `lk publish` reads it
//!   under;
//! - `LEPTOSKIT_PUBLISH_CONTENTS_ONLY=1`, so `lk publish` refuses a credential
//!   that names a file, which would be this machine's;
//! - `HOME` and `TMPDIR`: empty directories deleted with the source;
//! - `PATH`, and any variable the operator named to pass on.
//!
//! So the simplest program is
//!
//! ```sh
//! #!/bin/sh
//! submit=; [ "$LEPTOSKIT_PUBLISH_SUBMIT" = 1 ] && submit=--submit
//! exec lk publish $LEPTOSKIT_PUBLISH_STORES --json --yes $submit > "$LEPTOSKIT_PUBLISH_OUTPUT"
//! ```
//!
//! It succeeds when it exits with 0 and writes a report; anything else
//! fails, and its log says why. What it prints is the publish's log, with
//! each credential's value hidden. It is stopped as a build is: see
//! [`crate::command`]. Nothing here isolates it from the machine, and it
//! runs the organisation's own code with the organisation's credentials:
//! see the crate's documentation.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::workers::LeasedSecrets;
use leptoskit_cloud::workers::PublishInput;
use leptoskit_cloud::workers::PublishOutcome;
use leptoskit_cloud::workers::PublishReport;
use leptoskit_cloud::workers::is_publish_credential;
use serde::Deserialize;
use serde::Serialize;

use crate::build::Stop;
use crate::command::Exit;
use crate::command::Launch;
use crate::command::Redactions;
use crate::logs::LogSink;

/// The longest a failure's reason is.
const MAX_REASON: usize = 500;

/// What the program is asked to publish: a job's input without the
/// archive's address, which is a credential the program has no use for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishRequest {
	pub organization: String,
	pub project: String,
	pub environment: String,
	/// The source archive's SHA-256.
	pub digest: Digest,
	pub toolchain: String,
	pub stores: Vec<String>,
	pub submit: bool,
}

impl From<&PublishInput> for PublishRequest {
	fn from(input: &PublishInput) -> Self {
		Self {
			organization: input.organization.clone(),
			project: input.project.clone(),
			environment: input.environment.clone(),
			digest: input.digest.clone(),
			toolchain: input.toolchain.clone(),
			stores: input.stores.clone(),
			submit: input.submit,
		}
	}
}

/// One publish, as a [`CommandPublisher`] is handed it.
pub(crate) struct PublishJob<'a> {
	/// What to publish. Its `archive` is already fetched: `source` holds it.
	pub(crate) input: &'a PublishInput,
	pub(crate) source: &'a Path,
	pub(crate) scratch: &'a Path,
	/// The credentials, leased for this attempt.
	pub(crate) credentials: LeasedSecrets,
	pub(crate) log: LogSink,
	pub(crate) stop: Stop,
}

/// Publishes by running a program.
#[derive(Clone, Debug)]
pub struct CommandPublisher {
	program: PathBuf,
	arguments: Vec<String>,
	/// Names of the worker's own variables the program is given too.
	pass: Vec<String>,
	timeout: Duration,
}

impl CommandPublisher {
	/// Runs `program` with `arguments` for each publish, for two hours at
	/// most.
	#[must_use]
	pub fn new(program: impl Into<PathBuf>, arguments: Vec<String>) -> Self {
		Self {
			program: program.into(),
			arguments,
			pass: Vec::new(),
			timeout: Duration::from_hours(2),
		}
	}

	/// Also gives the program the worker's own variables named `names`.
	#[must_use]
	pub fn pass(mut self, names: Vec<String>) -> Self {
		self.pass = names;
		self
	}

	/// Stops a publish that runs longer than `timeout`.
	#[must_use]
	pub fn timeout(mut self, timeout: Duration) -> Self {
		self.timeout = timeout;
		self
	}

	pub(crate) async fn publish(&self, job: PublishJob<'_>) -> PublishOutcome {
		let PublishJob {
			input,
			source,
			scratch,
			credentials,
			log,
			stop,
		} = job;

		// Only what a store reads, whatever the control plane sent: a name
		// such as `PATH` or `LD_PRELOAD` would reach the program itself.
		if let Some(name) = credentials
			.names()
			.find(|name| !is_publish_credential(name))
		{
			return failed(format!("`{name}` isn't a store's credential"));
		}

		let request = scratch.join("publish.json");
		let output = scratch.join("report.json");
		let Ok((home, temporary)) =
			crate::command::prepare(scratch, &request, &PublishRequest::from(input)).await
		else {
			return failed("the worker couldn't prepare the publish's directory");
		};
		let values = credentials.into_values();
		let redact = Arc::new(Redactions::new(
			values
				.iter()
				.map(|(name, value)| (name.as_str(), value.as_str())),
		));
		let mut env = vec![
			("LEPTOSKIT_PUBLISH_SOURCE".to_owned(), source.into()),
			("LEPTOSKIT_PUBLISH_INPUT".to_owned(), request.into()),
			("LEPTOSKIT_PUBLISH_OUTPUT".to_owned(), output.clone().into()),
			(
				"LEPTOSKIT_PUBLISH_STORES".to_owned(),
				input.stores.join(" ").into(),
			),
			(
				"LEPTOSKIT_PUBLISH_SUBMIT".to_owned(),
				if input.submit { "1" } else { "0" }.into(),
			),
			// A credential that may name a file must hold its contents:
			// the file would be this machine's.
			("LEPTOSKIT_PUBLISH_CONTENTS_ONLY".to_owned(), "1".into()),
			("HOME".to_owned(), home.into()),
			("TMPDIR".to_owned(), temporary.into()),
		];
		env.extend(values.into_iter().map(|(name, value)| (name, value.into())));
		let exit = Launch {
			program: &self.program,
			arguments: &self.arguments,
			pass: &self.pass,
			timeout: self.timeout,
			source,
			env,
			log,
			stop,
			redact: Arc::clone(&redact),
			what: "publish",
		}
		.execute()
		.await;
		let status = match exit {
			Exit::Exited(status) => status,
			Exit::Stopped => return PublishOutcome::Stopped,
			Exit::TimedOut { minutes } => {
				return failed(format!(
					"the publish ran past its limit of {minutes} min; it may have uploaded to a \
					 store before it stopped"
				));
			}
			Exit::Unstarted => return failed("the worker couldn't start its publish command"),
		};

		match status {
			Ok(status) if status.success() => {
				match report(&output, &input.stores, &redact).await {
					Some(report) => PublishOutcome::Published(report),
					None => failed("the publish command ended without writing its report"),
				}
			}
			Ok(status) => {
				failed(match status.code() {
					Some(code) => {
						format!("the publish command exited with status {code}; its log says why")
					}
					None => "the publish command was killed".to_owned(),
				})
			}
			Err(_) => failed("the worker lost track of its publish command"),
		}
	}
}

fn failed(reason: impl Into<String>) -> PublishOutcome {
	let reason: String = reason
		.into()
		.chars()
		.map(|c| if c.is_control() { ' ' } else { c })
		.collect();
	let mut end = reason.len().min(MAX_REASON);

	while !reason.is_char_boundary(end) {
		end -= 1;
	}

	PublishOutcome::Failed {
		reason: reason[..end].to_owned(),
	}
}

/// The report the program wrote at `path`, held to its bounds, about
/// `stores` only, with every credential hidden.
async fn report(path: &Path, stores: &[String], redact: &Redactions) -> Option<PublishReport> {
	let bytes = crate::command::written(path).await?;
	let mut report = serde_json::from_slice::<PublishReport>(&bytes)
		.ok()?
		.bounded(stores);
	let hide = |text: &mut String| *text = redact.apply(text);

	for status in &mut report.stores {
		hide(&mut status.summary);
	}

	for step in &mut report.steps {
		hide(&mut step.what);
		hide(&mut step.why);
		hide(&mut step.done_when);

		if let Some(url) = &mut step.url {
			hide(url);
		}

		for field in &mut step.enter {
			hide(&mut field.label);
			hide(&mut field.value);
		}
	}

	Some(report)
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use leptoskit_cloud::workers::Download;

	use super::*;

	const KEY: &str = "LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET";
	const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

	fn input() -> PublishInput {
		PublishInput {
			organization: "ana".to_owned(),
			project: "notes".to_owned(),
			environment: "production".to_owned(),
			archive: Download {
				url: "https://store.example.test/secret-address".to_owned(),
				expires_at: "2026-11-19T00:00:00Z".parse().expect("a time"),
			},
			digest: Digest::of(b"source"),
			size: 6,
			toolchain: "stable".to_owned(),
			stores: vec!["chrome".to_owned(), "firefox".to_owned()],
			submit: false,
		}
	}

	/// What a publish by `script`, a shell script, ended as, and its log.
	async fn publish(
		script: &str,
		credentials: &[(&str, &str)],
	) -> (PublishOutcome, Vec<String>, tempfile::TempDir) {
		let directory = tempfile::tempdir().expect("a directory");
		let source = directory.path().join("source");
		let scratch = directory.path().join("scratch");
		std::fs::create_dir_all(&source).expect("a source");
		std::fs::create_dir_all(&scratch).expect("a scratch");
		let (log, mut lines) = LogSink::collecting();
		let input = input();
		let outcome = CommandPublisher::new("/bin/sh", vec!["-c".to_owned(), script.to_owned()])
			.publish(PublishJob {
				input: &input,
				source: &source,
				scratch: &scratch,
				credentials: LeasedSecrets::new(
					credentials
						.iter()
						.map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
						.collect::<BTreeMap<_, _>>(),
				),
				log,
				stop: Stop::never(),
			})
			.await;
		let mut log = Vec::new();

		while let Ok(line) = lines.try_recv() {
			log.push(line);
		}

		(outcome, log, directory)
	}

	#[tokio::test]
	async fn a_publish_gets_its_stores_and_credentials_and_answers_with_a_report() {
		let script = format!(
			"test \"$PWD\" = \"$LEPTOSKIT_PUBLISH_SOURCE\" && test \"$LEPTOSKIT_PUBLISH_STORES\" \
			 = 'chrome firefox' && test \"$LEPTOSKIT_PUBLISH_SUBMIT\" = 0 && ! grep -q \
			 secret-address \"$LEPTOSKIT_PUBLISH_INPUT\" && echo \"signing with ${KEY}\" && cat > \
			 \"$LEPTOSKIT_PUBLISH_OUTPUT\" \
			 <<JSON\n{{\"stores\":[{{\"store\":\"firefox\",\"summary\":\"uploaded with \
			 ${KEY}\"}},{{\"store\":\"steam\",\"summary\":\"not \
			 asked\"}}],\"steps\":[{{\"store\":\"firefox\",\"by\":\"person\",\"what\":\"Accept\",\\
			 "why\":\"policy\",\"done_when\":\"listed\"}}]}}\nJSON"
		);
		let (outcome, log, _directory) = publish(&script, &[(KEY, SECRET)]).await;
		let PublishOutcome::Published(report) = outcome else {
			panic!("{outcome:?} {log:?}");
		};
		assert_eq!(log, ["signing with [redacted]"]);
		assert_eq!(report.stores.len(), 1, "only the stores it was asked for");
		assert_eq!(report.stores[0].summary, "uploaded with [redacted]");
		assert_eq!(report.steps[0].what, "Accept");
	}

	#[tokio::test]
	async fn a_publish_that_fails_or_writes_nothing_says_so() {
		let (outcome, _, _directory) = publish("echo nope >&2; exit 4", &[]).await;
		assert_eq!(
			outcome,
			PublishOutcome::Failed {
				reason: "the publish command exited with status 4; its log says why".to_owned()
			}
		);
		let (outcome, _, _directory) = publish("true", &[]).await;
		assert!(
			matches!(&outcome, PublishOutcome::Failed { reason } if reason.contains("without writing")),
			"{outcome:?}"
		);
	}

	#[tokio::test]
	async fn only_a_stores_credential_reaches_the_program() {
		let (outcome, log, _directory) = publish(
			"echo ran; echo '{\"stores\":[]}' > \"$LEPTOSKIT_PUBLISH_OUTPUT\"",
			&[("LD_PRELOAD", "/tmp/evil.so")],
		)
		.await;
		assert!(log.is_empty(), "{log:?}");
		assert!(
			matches!(&outcome, PublishOutcome::Failed { reason } if reason.contains("LD_PRELOAD")),
			"{outcome:?}"
		);
	}

	#[test]
	fn a_credential_cut_across_lines_is_still_hidden() {
		let key = "-----BEGIN PRIVATE \
		           KEY-----\nMIGTAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBHkwdwIBAQQg\n-----END PRIVATE \
		           KEY-----";
		let redactions = Redactions::new([
			("LEPTOSKIT_PUBLISH_APPLE_KEY", key),
			("LEPTOSKIT_ANDROID_KEY_ALIAS", "short"),
			("LEPTOSKIT_ANDROID_KEY_PASSWORD", "hunt"),
		]);
		// A password is hidden however short.
		assert_eq!(redactions.apply("password hunt"), "password [redacted]");
		assert_eq!(
			redactions.apply("key: MIGTAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBHkwdwIBAQQg"),
			"key: [redacted]"
		);
		// A piece of the run, as a cut line leaves it.
		assert_eq!(redactions.apply("…MIGTAgEAMBMGByqGSM49Ag"), "…[redacted]");
		// Short values and ordinary words stay.
		assert_eq!(
			redactions.apply("short words stay PRIVATE"),
			"short words stay PRIVATE"
		);
	}
}
