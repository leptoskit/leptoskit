//! The operator's push configuration: which transports this service
//! speaks, from its environment, and what each provider needs to sign
//! with. A service `lk` starts gets a cleared environment and only the
//! variables its `[services.notifications]` table lists under `secrets`,
//! so the guide shows that list; the crate's own manifest names none,
//! because a deploy requires every listed secret and most apps configure
//! one or two transports, not all three.
//!
//! Every variable is operator-provided secret material, read once at
//! startup and held only in the transports' parsed keys. A provider with
//! some of its variables set and the rest missing refuses to start with
//! every missing or unusable piece named: half-configured is worse than
//! unconfigured, because it would look like push works until the first
//! send. A provider with nothing set is unconfigured, which is a normal
//! way to run — its platform is refused at enrollment and its inbox is
//! untouched.
//!
//! Beside the secrets sit two plain settings, read the same way, once at
//! startup: [`OpenTopics`], the topic patterns a signed-in person may
//! subscribe themselves to — nothing being open is the default — and
//! [`Retention`], how long the service keeps what it no longer needs.

/// APNs's two servers: the one the service uses for an endpoint whose
/// device reported no environment of its own.
pub use crate::push::ApnsEnvironment;

/// The APNs transport's configuration: the operator's `.p8` key, the ids
/// that name it, the app's topic, and which server to talk to.
#[derive(Clone)]
pub struct ApnsConfig {
	/// The `.p8` file's contents, PEM.
	pub key_p8: String,
	/// The key's id, from the same developer account page.
	pub key_id: String,
	/// The team that owns the key.
	pub team_id: String,
	/// The app's bundle id: the `apns-topic` every send carries.
	pub topic: String,
	/// Which server pushes go to.
	pub environment: ApnsEnvironment,
}

impl std::fmt::Debug for ApnsConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ApnsConfig")
			.field("key_p8", &"<redacted>")
			.field("key_id", &self.key_id)
			.field("team_id", &self.team_id)
			.field("topic", &self.topic)
			.field("environment", &self.environment)
			.finish()
	}
}

/// The FCM transport's configuration: the operator's service-account
/// JSON file, whose project the sends go out under and whose key signs
/// the token exchange.
#[derive(Clone)]
pub struct FcmConfig {
	/// The service account JSON file's contents.
	pub service_account_json: String,
}

impl std::fmt::Debug for FcmConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("FcmConfig")
			.field("service_account_json", &"<redacted>")
			.finish()
	}
}

/// The Web Push transport's configuration: the VAPID key pair the
/// ecosystem's own tools generate, the contact push services may write
/// to, and any extra push-service hosts the operator approved.
#[derive(Clone)]
pub struct WebPushConfig {
	/// The private key, 32 bytes, base64url.
	pub vapid_private_key: String,
	/// The public key, 65 bytes, base64url.
	pub vapid_public_key: String,
	/// A `mailto:` or `https:` contact.
	pub subject: String,
	/// Push-service hosts beyond the shipped ones, exact names, pushed
	/// to only over DNS-validated pinned connections.
	pub extra_hosts: Vec<String>,
}

impl std::fmt::Debug for WebPushConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WebPushConfig")
			.field("vapid_private_key", &"<redacted>")
			.field("vapid_public_key", &self.vapid_public_key)
			.field("subject", &self.subject)
			.field("extra_hosts", &self.extra_hosts)
			.finish()
	}
}

/// The operator's push configuration, provider by provider. `None` is
/// unconfigured, which the enrollment and delivery paths answer clearly.
#[derive(Clone, Debug, Default)]
pub struct PushConfig {
	pub apns: Option<ApnsConfig>,
	pub fcm: Option<FcmConfig>,
	pub web: Option<WebPushConfig>,
}

impl PushConfig {
	/// Reads the operator's environment. Every variable's name is
	/// `LEPTOSKIT_NOTIFICATIONS_` plus its provider's part, exactly as the
	/// guide documents; nothing here resolves a secret, only reads what
	/// the process was given.
	///
	/// # Errors
	///
	/// Every missing or unusable piece of a partially-set provider,
	/// named, so an operator sees the whole fix at once.
	pub fn from_env() -> Result<Self, String> {
		Self::from_vars(std::env::vars())
	}

	/// Reads the operator's configuration from `vars` instead of the
	/// process's, as a test does.
	///
	/// # Errors
	///
	/// As [`from_env`](Self::from_env).
	pub fn from_vars<K: AsRef<str>, V: AsRef<str>>(
		vars: impl IntoIterator<Item = (K, V)>,
	) -> Result<Self, String> {
		let mut found: std::collections::HashMap<&'static str, String> =
			std::collections::HashMap::new();

		let names = [
			"APNS_KEY_P8",
			"APNS_KEY_ID",
			"APNS_TEAM_ID",
			"APNS_TOPIC",
			"APNS_ENVIRONMENT",
			"FCM_SERVICE_ACCOUNT",
			"WEB_PUSH_VAPID_PRIVATE_KEY",
			"WEB_PUSH_VAPID_PUBLIC_KEY",
			"WEB_PUSH_VAPID_SUBJECT",
			"WEB_PUSH_EXTRA_HOSTS",
		];

		for (key, value) in vars {
			let key = key.as_ref();
			let value = value.as_ref();

			for name in names {
				if key == format!("LEPTOSKIT_NOTIFICATIONS_{name}") {
					found.insert(name, value.to_owned());
				}
			}
		}

		let mut take =
			|name: &'static str| found.remove(name).filter(|value| !value.trim().is_empty());

		let apns = required(
			"apns",
			&[
				("APNS_KEY_P8", take("APNS_KEY_P8")),
				("APNS_KEY_ID", take("APNS_KEY_ID")),
				("APNS_TEAM_ID", take("APNS_TEAM_ID")),
				("APNS_TOPIC", take("APNS_TOPIC")),
				("APNS_ENVIRONMENT", take("APNS_ENVIRONMENT")),
			],
		)?
		.map(|values| {
			let environment = match values["APNS_ENVIRONMENT"].trim() {
				"sandbox" => ApnsEnvironment::Sandbox,
				"production" => ApnsEnvironment::Production,
				other => {
					return Err(format!(
						"LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT is {other:?}: use `sandbox` or \
						 `production`"
					));
				}
			};

			Ok::<ApnsConfig, String>(ApnsConfig {
				key_p8: values["APNS_KEY_P8"].clone(),
				key_id: values["APNS_KEY_ID"].clone(),
				team_id: values["APNS_TEAM_ID"].clone(),
				topic: values["APNS_TOPIC"].clone(),
				environment,
			})
		})
		.transpose()?;

		let fcm = required(
			"fcm",
			&[("FCM_SERVICE_ACCOUNT", take("FCM_SERVICE_ACCOUNT"))],
		)?
		.map(|values| {
			FcmConfig {
				service_account_json: values["FCM_SERVICE_ACCOUNT"].clone(),
			}
		});

		let vapid = [
			(
				"WEB_PUSH_VAPID_PRIVATE_KEY",
				take("WEB_PUSH_VAPID_PRIVATE_KEY"),
			),
			(
				"WEB_PUSH_VAPID_PUBLIC_KEY",
				take("WEB_PUSH_VAPID_PUBLIC_KEY"),
			),
			("WEB_PUSH_VAPID_SUBJECT", take("WEB_PUSH_VAPID_SUBJECT")),
		];
		let extra_hosts = take("WEB_PUSH_EXTRA_HOSTS");

		// The extra hosts are optional beside a VAPID pair, and half a
		// transport without one: they say where pushes may go, not how
		// to sign them.
		if extra_hosts.is_some() && vapid.iter().all(|(_, value)| value.is_none()) {
			return Err(half_configured("web push", &vapid));
		}

		let web = required("web push", &vapid)?.map(|values| {
			let extra_hosts = extra_hosts
				.unwrap_or_default()
				.split(',')
				.map(str::trim)
				.filter(|host| !host.is_empty())
				.map(str::to_owned)
				.collect();

			WebPushConfig {
				vapid_private_key: values["WEB_PUSH_VAPID_PRIVATE_KEY"].clone(),
				vapid_public_key: values["WEB_PUSH_VAPID_PUBLIC_KEY"].clone(),
				subject: values["WEB_PUSH_VAPID_SUBJECT"].clone(),
				extra_hosts,
			}
		});

		Ok(Self { apns, fcm, web })
	}
}

/// Whether a provider's variables are set whole: all of them, or none of
/// them. Some of them is an error naming every missing piece.
///
/// # Errors
///
/// The provider's missing variables, named with their full environment
/// names.
fn required(
	provider: &str,
	variables: &[(&'static str, Option<String>)],
) -> Result<Option<std::collections::HashMap<&'static str, String>>, String> {
	let set = variables
		.iter()
		.filter(|(_, value)| value.is_some())
		.count();

	if set == 0 {
		return Ok(None);
	}

	if set == variables.len() {
		let values = variables
			.iter()
			.filter_map(|(name, value)| value.as_ref().map(|value| (*name, value.to_owned())))
			.collect::<std::collections::HashMap<_, _>>();

		return Ok(Some(values));
	}

	Err(half_configured(provider, variables))
}

/// The refusal for a provider with some of its variables set: every
/// missing one, named in full.
fn half_configured(provider: &str, variables: &[(&'static str, Option<String>)]) -> String {
	let missing = variables
		.iter()
		.filter(|(_, value)| value.is_none())
		.map(|(name, _)| format!("LEPTOSKIT_NOTIFICATIONS_{name}"))
		.collect::<Vec<_>>()
		.join(", ");

	format!(
		"the {provider} push transport is half configured: set all of its variables or none. \
		 Missing: {missing}"
	)
}

/// The topics a signed-in person may subscribe themselves to: patterns
/// like `news.*`, read from `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS`,
/// comma-separated. Nothing is open by default — a topic a person may
/// join is the operator's deliberate publication, not the service's
/// guess — and every topic is open to the app's own batch calls
/// regardless.
///
/// A pattern is a topic name with `*` standing for any run of the
/// characters a topic name allows, including none: `news.*` matches
/// `news.breaking` and `news.`, but not `news` itself. The patterns are
/// validated once at startup, so a typo refuses to start named rather
/// than quietly closing or opening topics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenTopics {
	patterns: Vec<String>,
}

impl OpenTopics {
	/// Reads the operator's environment.
	///
	/// # Errors
	///
	/// Every unusable pattern, named, so an operator sees the whole fix
	/// at once.
	pub fn from_env() -> Result<Self, String> {
		Self::from_vars(std::env::vars())
	}

	/// Reads the operator's configuration from `vars` instead of the
	/// process's, as a test does.
	///
	/// # Errors
	///
	/// As [`from_env`](Self::from_env).
	pub fn from_vars<K: AsRef<str>, V: AsRef<str>>(
		vars: impl IntoIterator<Item = (K, V)>,
	) -> Result<Self, String> {
		let raw = vars
			.into_iter()
			.find(|(key, _)| key.as_ref() == "LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS")
			.map(|(_, value)| value.as_ref().to_owned())
			.unwrap_or_default();

		let patterns = raw
			.split(',')
			.map(str::trim)
			.filter(|pattern| !pattern.is_empty())
			.map(|pattern| {
				if pattern.len() > crate::MAX_TOPIC_LEN
					|| !pattern
						.bytes()
						.all(|byte| byte == b'*' || crate::topic_byte(byte))
				{
					return Err(format!(
						"LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS holds `{pattern}`, which isn't a \
						 topic pattern: topic characters with `*` as the wildcard"
					));
				}

				Ok(pattern.to_owned())
			})
			.collect::<Result<Vec<_>, _>>()?;

		Ok(Self { patterns })
	}

	/// The patterns the operator declared, as written.
	#[must_use]
	pub fn patterns(&self) -> &[String] {
		&self.patterns
	}

	/// Whether `topic` matches one of them.
	#[must_use]
	pub fn is_open(&self, topic: &str) -> bool {
		self.patterns.iter().any(|pattern| matches(pattern, topic))
	}
}

/// How long the service keeps what it no longer needs, in whole days:
/// what the pruning job ([`prune`](crate::prune)) deletes, an hour at a
/// time. Read from `LEPTOSKIT_NOTIFICATIONS_RETENTION_*`; each value is a
/// number of days, or `off` to keep that kind of row forever. A value
/// that is neither refuses to start, named, like every other setting
/// here.
///
/// The defaults keep a year of inbox, a month of what nobody can see or
/// act on any more, and three months of send records. Nothing is deleted
/// that the service still works with: a notification whose push is still
/// queued, a send still fanning out, or a send record whose notifications
/// are still stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
	/// How long a notification stays in an inbox, counted from the moment
	/// it became visible, read or not: `RETENTION_INBOX_DAYS`, 365 by
	/// default. A scheduled notification is never old before its time.
	pub inbox_days: Option<u32>,
	/// How long a notification nobody can see any more is kept, counted
	/// from the moment it ended — its expiry, its cancellation, or, for a
	/// silent data message, the end of its delivery window:
	/// `RETENTION_ENDED_DAYS`, 30 by default. A message cancelled before
	/// the time it was scheduled for is counted from that time.
	pub ended_days: Option<u32>,
	/// How long a finished push delivery — accepted, rejected, or skipped
	/// — stays readable through
	/// [`push_deliveries`](crate::push::push_deliveries), counted from when
	/// it finished: `RETENTION_DELIVERIES_DAYS`, 30 by default. A delivery
	/// never outlives its notification.
	pub deliveries_days: Option<u32>,
	/// How long a send's record is kept after the send was made:
	/// `RETENTION_SENDS_DAYS`, 90 by default, and at least
	/// [`MIN_SENDS_DAYS`](Self::MIN_SENDS_DAYS). The record is what
	/// recognises a repeated idempotency key, so this is how long a retry
	/// is safe; it is also kept for as long as any notification the send
	/// wrote is still stored, however long that is.
	pub sends_days: Option<u32>,
}

impl Default for Retention {
	fn default() -> Self {
		Self {
			inbox_days: Some(365),
			ended_days: Some(30),
			deliveries_days: Some(30),
			sends_days: Some(90),
		}
	}
}

impl Retention {
	/// The shortest a send's record may be kept, in days. The jobs a send
	/// enqueues are keyed by its idempotency key, and the job table
	/// remembers a finished job's key for a day: a record gone sooner
	/// could let the same key be sent again while those keys still stand,
	/// and its fanout would never start.
	pub const MIN_SENDS_DAYS: u32 = 2;
	/// The longest any of these may be: a century.
	pub const MOST_DAYS: u32 = 36_500;

	/// Keeps everything forever: no pruning at all.
	#[must_use]
	pub fn forever() -> Self {
		Self {
			inbox_days: None,
			ended_days: None,
			deliveries_days: None,
			sends_days: None,
		}
	}

	/// Reads the operator's environment.
	///
	/// # Errors
	///
	/// Every unusable value, named, so an operator sees the whole fix at
	/// once.
	pub fn from_env() -> Result<Self, String> {
		Self::from_vars(std::env::vars())
	}

	/// Reads the operator's configuration from `vars` instead of the
	/// process's, as a test does.
	///
	/// # Errors
	///
	/// As [`from_env`](Self::from_env).
	pub fn from_vars<K: AsRef<str>, V: AsRef<str>>(
		vars: impl IntoIterator<Item = (K, V)>,
	) -> Result<Self, String> {
		let mut retention = Self::default();
		let mut problems = Vec::new();

		for (key, value) in vars {
			let Some(name) = key
				.as_ref()
				.strip_prefix("LEPTOSKIT_NOTIFICATIONS_RETENTION_")
			else {
				continue;
			};
			let (slot, least) = match name {
				"INBOX_DAYS" => (&mut retention.inbox_days, 1),
				"ENDED_DAYS" => (&mut retention.ended_days, 1),
				"DELIVERIES_DAYS" => (&mut retention.deliveries_days, 1),
				"SENDS_DAYS" => (&mut retention.sends_days, Self::MIN_SENDS_DAYS),
				_ => {
					problems.push(format!(
						"LEPTOSKIT_NOTIFICATIONS_RETENTION_{name} isn't a retention setting: the \
						 settings are INBOX_DAYS, ENDED_DAYS, DELIVERIES_DAYS, and SENDS_DAYS"
					));
					continue;
				}
			};
			let value = value.as_ref().trim();

			// Set but empty is unset, as it is for every other variable here.
			if value.is_empty() {
				continue;
			}

			if value == "off" {
				*slot = None;
				continue;
			}

			match value.parse::<u32>() {
				Ok(days) if (least..=Self::MOST_DAYS).contains(&days) => *slot = Some(days),
				_ => {
					problems.push(format!(
						"LEPTOSKIT_NOTIFICATIONS_RETENTION_{name} is {value:?}: use a whole \
						 number of days from {least} to {}, or `off` to keep these rows forever",
						Self::MOST_DAYS
					));
				}
			}
		}

		if problems.is_empty() {
			Ok(retention)
		} else {
			problems.sort();
			Err(problems.join("; "))
		}
	}

	/// The settings as one line, for the service's startup log.
	#[must_use]
	pub fn describe(&self) -> String {
		let days = |days: Option<u32>| {
			days.map_or_else(|| "kept forever".to_owned(), |days| format!("{days} days"))
		};

		format!(
			"inbox {}, ended notifications {}, finished deliveries {}, send records {}",
			days(self.inbox_days),
			days(self.ended_days),
			days(self.deliveries_days),
			days(self.sends_days)
		)
	}
}

/// Whether `topic` fits `pattern`: the literal parts in order, each `*`
/// any run of topic characters, including none.
fn matches(pattern: &str, topic: &str) -> bool {
	let mut stars = pattern.split('*');
	let first = stars.next().unwrap_or_default();

	if !topic.starts_with(first) {
		return false;
	}

	let mut rest = &topic[first.len()..];
	let segments: Vec<&str> = stars.collect();
	let Some((&last, middles)) = segments.split_last() else {
		// No `*` anywhere: the whole pattern had to be the topic.
		return rest.is_empty();
	};

	for middle in middles {
		let Some(at) = rest.find(middle) else {
			return false;
		};

		rest = &rest[at + middle.len()..];
	}

	rest.ends_with(last)
}

#[cfg(test)]
mod open_topics_tests {
	use super::OpenTopics;

	fn open(patterns: &str) -> OpenTopics {
		OpenTopics::from_vars([("LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS", patterns)]).unwrap()
	}

	#[test]
	fn nothing_is_open_by_default() {
		let none = OpenTopics::from_vars(Vec::<(String, String)>::new()).unwrap();
		assert!(!none.is_open("news"), "no variable at all opens nothing");
		assert!(!open("").is_open("news"), "an empty value opens nothing");
		assert!(
			none.patterns().is_empty(),
			"no variable at all opens nothing"
		);
	}

	#[test]
	fn a_star_matches_any_run_of_topic_characters() {
		let topics = open("news.*, alerts.*");
		assert!(topics.is_open("news.breaking"));
		assert!(topics.is_open("news."));
		assert!(!topics.is_open("news"));
		assert!(!topics.is_open("offers.local"));
		assert!(topics.is_open("alerts.local"), "the second pattern counts");

		assert!(open("a*b").is_open("ab"), "even nothing at all");
		assert!(open("a*b").is_open("a-middle-b"));
		assert!(!open("a*b").is_open("a-middle-c"));

		// A `*` in the middle narrows both sides.
		assert!(open("news.*.major").is_open("news.breaking.major"));
		assert!(!open("news.*.major").is_open("news.major"));
	}

	#[test]
	fn an_unusable_pattern_refuses_to_start_named() {
		for bad in ["news/child", "a b", "*é", "news (major)"] {
			let error = OpenTopics::from_vars([("LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS", bad)])
				.expect_err(bad);
			assert!(
				error.contains(bad) && error.contains("topic pattern"),
				"names the pattern and the rule: {error}"
			);
		}
	}
}

#[cfg(test)]
mod retention_tests {
	use super::Retention;

	fn read(vars: &[(&str, &str)]) -> Result<Retention, String> {
		Retention::from_vars(vars.iter().copied())
	}

	#[test]
	fn the_defaults_keep_a_year_of_inbox_and_three_months_of_sends() {
		let retention = read(&[]).unwrap();
		assert_eq!(retention, Retention::default());
		assert_eq!(retention.inbox_days, Some(365));
		assert_eq!(retention.ended_days, Some(30));
		assert_eq!(retention.deliveries_days, Some(30));
		assert_eq!(retention.sends_days, Some(90));
		assert_eq!(
			retention.describe(),
			"inbox 365 days, ended notifications 30 days, finished deliveries 30 days, send \
			 records 90 days"
		);
	}

	#[test]
	fn each_setting_reads_days_or_off() {
		let retention = read(&[
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS", "off"),
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_ENDED_DAYS", " 7 "),
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_DELIVERIES_DAYS", ""),
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS", "2"),
			("LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS", "news.*"),
		])
		.unwrap();

		assert_eq!(retention.inbox_days, None, "`off` keeps them forever");
		assert_eq!(retention.ended_days, Some(7));
		assert_eq!(retention.deliveries_days, Some(30), "empty is unset");
		assert_eq!(retention.sends_days, Some(2));
		assert!(retention.describe().starts_with("inbox kept forever"));
		assert_eq!(Retention::forever().sends_days, None);
	}

	#[test]
	fn an_unusable_value_refuses_to_start_named() {
		let error = read(&[
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS", "0"),
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_ENDED_DAYS", "a week"),
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS", "1"),
			("LEPTOSKIT_NOTIFICATIONS_RETENTION_SEND_DAYS", "30"),
		])
		.unwrap_err();

		for part in [
			"RETENTION_INBOX_DAYS is \"0\"",
			"RETENTION_ENDED_DAYS is \"a week\"",
			"RETENTION_SENDS_DAYS is \"1\": use a whole number of days from 2",
			"RETENTION_SEND_DAYS isn't a retention setting",
		] {
			assert!(error.contains(part), "names {part}: {error}");
		}
	}
}
