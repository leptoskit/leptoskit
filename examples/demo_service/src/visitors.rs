//! Browser-owned demo identities: the pass cookie, the pre-minted
//! people, the caps, and the ownership checks the sign-in gate reads.
//!
//! The rules the brief fixes, and how this module keeps them:
//!
//! - **A pass is this browser's only key.** The token is 32 random bytes; the
//!   database keeps only its domain-separated SHA-256, the optional salted IP
//!   hash, and the created time. The cookie is `HttpOnly`, `SameSite=Lax`,
//!   `Path=/`, never carries a `Domain`, is `Secure` with the `__Host-` prefix
//!   on HTTPS, and expires at the next UTC midnight — one day, hard-bounded.
//! - **A person is pre-minted, never chosen.** Only this server picks the
//!   persona, the synthetic `@demo.invalid` address, and the role. A pass owns
//!   at most two people, one Ada and one Grace, and a person claims its Auth
//!   account only once, by its already-minted address, inside the sign-up
//!   transaction.
//! - **Caps are counts in the database, not counters in memory.** The per-IP
//!   hourly window and the per-generation day cap are counted and enforced
//!   inside one immediate transaction, so concurrent requests and two service
//!   processes on the same volume cannot race past them. A database failure
//!   refuses the mint; it never bypasses the caps.
//! - **Client IPs are trusted from one place only**: the configured edge header
//!   when one is set — refusing malformed, duplicated, comma-chained, and
//!   non-IP values — or the socket peer when it is not. Visitor tables store a
//!   salted hash, never an address.
//!
//! No notes, events counters, or analytics emission live here: those
//! are later tasks, and this module gives them durable people to attach
//! to.

use std::net::IpAddr;
use std::net::SocketAddr;

use anyhow::Context as _;
use anyhow::Result;
use axum::http::HeaderMap;
use axum::http::header;
use leptoskit_auth::Account;
use leptoskit_auth::AuthError;
use leptoskit_auth::Keys;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::Result as AuthResult;
use leptoskit_auth::Roles;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Email;
use leptoskit_db::Enum;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use sha2::Digest as _;
use sha2::Sha256;

use crate::return_to;

/// The role the site's Ada persona carries, as the identity provider's
/// `roles` scope releases it to the admin.
pub(crate) const ADMIN_ROLE: &str = "admin";

/// New passes one canonical client IP may mint per rolling hour.
const NEW_PASSES_PER_IP_PER_HOUR: usize = 10;

/// New passes one generation mints per UTC day. This bounds the people
/// table (at most two people per pass, so up to 10,000 people), not the
/// population of visitors: minted passes keep signing in when the caps
/// are full.
const NEW_PASSES_PER_DAY: usize = 5_000;

/// How long the per-IP window looks back.
const IP_WINDOW: chrono::TimeDelta = chrono::TimeDelta::hours(1);

/// The bytes of randomness in a pass token: 32 bytes, hex-encoded.
const TOKEN_BYTES: usize = 32;

/// The hex characters in a minted email's random part.
const EMAIL_RANDOM_CHARS: usize = 16;

/// The domain separator for a pass token's stored hash.
const TOKEN_DOMAIN: &[u8] = b"leptoskit demo pass token";

/// The purpose the daily site key derives the IP-hash salt for.
const IP_KEY_PURPOSE: &[u8] = b"leptoskit demo visitor ip hash";

/// Who a visitor picked being. The closed set the demo offers.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persona {
	/// Runs the place: the admin's role gate admits her.
	#[db(rename = "ada")]
	Ada,
	/// Writes notes: the admin turns her away, and shows how.
	#[db(rename = "grace")]
	Grace,
}

impl Persona {
	/// The persona a form field names, and nothing else.
	pub(crate) fn of_form(value: &str) -> Option<Self> {
		match value.trim() {
			"ada" => Some(Self::Ada),
			"grace" => Some(Self::Grace),
			_ => None,
		}
	}

	/// The lowercase name the address and the JSON contract use.
	#[must_use]
	pub(crate) const fn slug(self) -> &'static str {
		match self {
			Self::Ada => "ada",
			Self::Grace => "grace",
		}
	}

	/// The name a person goes by, such as `Ada #7F3K`.
	#[must_use]
	pub fn name(self, tag: &str) -> String {
		match self {
			Self::Ada => format!("Ada #{tag}"),
			Self::Grace => format!("Grace #{tag}"),
		}
	}
}

/// One browser's pass: the durable half of its cookie.
#[table(name = "demo_passes", index(ip_hash))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct DemoPass {
	pub id: Id<Self>,
	/// The SHA-256 of the cookie's token over [`TOKEN_DOMAIN`]. The
	/// token itself is never stored anywhere.
	#[db(unique, max_len = 64)]
	pub token_hash: String,
	/// The salted hash of the canonical client IP that minted the pass,
	/// when one was known. The address itself is never stored.
	#[db(max_len = 64)]
	pub ip_hash: Option<String>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// One made-up person a pass owns. Minted before any account exists;
/// the sign-up hook claims it for the account its address proves.
#[table(name = "demo_people", index(pass_id), unique(pass_id, persona))]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct DemoPerson {
	pub id: Id<Self>,
	/// The pass this person belongs to. The person goes with it.
	#[db(on_delete = cascade)]
	pub pass_id: Id<DemoPass>,
	pub persona: Persona,
	/// The synthetic address, unique across every visitor: the only
	/// thing the sign-up hook matches on.
	pub email: Email,
	/// The Auth account the address proved, exactly once. `None` until
	/// the first successful verify.
	#[db(unique, on_delete = cascade)]
	pub account_id: Option<Id<Account>>,
	/// The latest mail receipt the mailer recorded: an opaque id and
	/// when it was delivered. The message itself stays in memory, so a
	/// receipt without a matching message means unavailable mail.
	#[db(max_len = 64)]
	pub last_receipt_id: Option<String>,
	pub last_receipt_at: Option<Timestamp>,
}

impl OnSignUp for DemoPerson {
	/// Claims the person this account's address was minted for, inside
	/// the sign-up's own transaction: the claim matches the minted
	/// address and an unclaimed row, updates exactly one, and refuses
	/// everything else. An address the demo never minted creates
	/// nothing.
	async fn on_sign_up(tx: &Tx, account: Id<Account>, profile: &Profile) -> AuthResult<()> {
		let Some(email) = profile.email.as_ref() else {
			return Err(AuthError::NotAllowed);
		};

		let claimed = DemoPerson::update()
			.filter(|person| person.email.eq(email.clone()) & person.account_id.is_null())
			.set(|person| person.account_id, Some(account))
			.run(tx)
			.await?;

		if claimed == 0 {
			return Err(AuthError::NotAllowed);
		}

		Ok(())
	}
}

impl Roles for DemoPerson {
	/// The persona's role, read by the account id's unique index. Only
	/// this server chooses roles; a visitor never sends one.
	async fn roles(db: &Db, account: Id<Account>) -> AuthResult<Vec<String>> {
		let person = DemoPerson::query()
			.filter(|person| person.account_id.eq(account))
			.first(db)
			.await?;

		Ok(match person {
			Some(person) if person.persona == Persona::Ada => vec![ADMIN_ROLE.to_owned()],
			_ => Vec::new(),
		})
	}
}

/// The caps a site's minting runs under. Production values are
/// constants; only a debug build's tests may narrow them, and a release
/// binary never sees the override.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caps {
	pub(crate) new_passes_per_ip_per_hour: u64,
	pub(crate) new_passes_per_day: u64,
}

impl Default for Caps {
	fn default() -> Self {
		Self {
			new_passes_per_ip_per_hour: u64::try_from(NEW_PASSES_PER_IP_PER_HOUR)
				.expect("the hourly cap fits a u64"),
			new_passes_per_day: u64::try_from(NEW_PASSES_PER_DAY)
				.expect("the daily cap fits a u64"),
		}
	}
}

/// What the pass cookie said, after this generation checked it.
#[derive(Debug)]
pub(crate) enum PassState {
	/// The cookie names a pass this generation minted.
	Valid(DemoPass),
	/// No cookie, an unusable one, or a token this generation never
	/// minted. The cookie is cleared.
	Absent,
	/// The cookie names an earlier day: the demo reset since. The
	/// cookie is cleared, and the response contract names this state so
	/// the front door can explain the new day.
	EarlierDay,
}

impl PassState {
	/// Whether a valid pass was resolved.
	pub(crate) const fn is_valid(&self) -> bool {
		matches!(self, Self::Valid(_))
	}
}

/// Why a new pass was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MintError {
	/// Too many new passes from one canonical IP this hour.
	#[error("too many passes from this address; try again later")]
	IpLimited {
		/// How long until the oldest pass leaves the window.
		retry_after: chrono::TimeDelta,
	},
	/// The generation minted its day's passes. Existing passes keep
	/// working.
	#[error("today's passes are all taken")]
	Full,
	/// The database refused the minted state: an explicit failure,
	/// never a bypassed cap.
	#[error("the database refused the mint: {0}")]
	Database(#[source] anyhow::Error),
}

impl From<DbError> for MintError {
	fn from(error: DbError) -> Self {
		Self::Database(error.into())
	}
}

impl From<anyhow::Error> for MintError {
	fn from(error: anyhow::Error) -> Self {
		Self::Database(error)
	}
}

/// Why a request's client IP could not be read.
#[derive(Debug, thiserror::Error)]
#[error("the client IP header is unusable")]
pub(crate) struct ClientIpError;

/// The cookie the pass travels in, per host kind.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PassCookie {
	/// `__Host-demo-pass` on HTTPS origins, `demo-pass` on plain
	/// loopback HTTP.
	pub name: &'static str,
	/// Whether the cookie must carry `Secure`.
	pub secure: bool,
}

impl PassCookie {
	/// The cookie for a site origin, the same rule the session cookie
	/// follows: `__Host-` and `Secure` unless the origin is plain HTTP.
	#[must_use]
	pub fn of_origin(origin: &str) -> Self {
		let secure = !origin.starts_with("http://");
		Self {
			name: if secure {
				"__Host-demo-pass"
			} else {
				"demo-pass"
			},
			secure,
		}
	}

	/// The `Set-Cookie` value that carries a pass until the next UTC
	/// midnight. `HttpOnly`, `Path=/`, `SameSite=Lax`, `Secure` on
	/// HTTPS, never a `Domain`.
	#[must_use]
	pub fn set(&self, value: &str, max_age: i64) -> axum::http::HeaderValue {
		let secure = if self.secure { "; Secure" } else { "" };
		axum::http::HeaderValue::from_str(&format!(
			"{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}",
			self.name
		))
		.expect("a pass cookie value is header-safe")
	}

	/// The `Set-Cookie` value that removes the cookie.
	#[must_use]
	pub fn clear(&self) -> axum::http::HeaderValue {
		self.set("", 0)
	}
}

/// The raw pass cookie a request carried, if one did.
#[must_use]
pub(crate) fn carried_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
	headers
		.get_all(header::COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(';'))
		.filter_map(|pair| pair.trim().split_once('='))
		.find_map(|(key, value)| (key == name && !value.is_empty()).then(|| value.to_owned()))
}

/// Seconds from `now` to the next UTC midnight, always at least one.
#[must_use]
pub(crate) fn seconds_until_next_midnight(now: chrono::DateTime<chrono::Utc>) -> i64 {
	let tomorrow = (now.date_naive() + chrono::Days::new(1))
		.and_hms_opt(0, 0, 0)
		.expect("midnight exists");
	(tomorrow.and_utc() - now).num_seconds().max(1)
}

/// The client IP a request may be judged by: the configured edge header
/// when one is set — exactly one header line, one address, no comma
/// chain — or the socket peer when no header is configured. Anything
/// else is an error, never an unlimited caller.
pub(crate) fn client_ip(
	header_name: Option<&str>,
	headers: &HeaderMap,
	peer: Option<SocketAddr>,
) -> Result<Option<IpAddr>, ClientIpError> {
	let Some(name) = header_name else {
		return Ok(peer.map(|peer| peer.ip()));
	};

	let mut values = headers.get_all(name).iter();
	let Some(value) = values.next() else {
		// No header line: no address, the same answer a peer-less local
		// process gives. The edge always sets the header; Auth skips its
		// own IP bucket in exactly this case.
		return Ok(None);
	};

	if values.next().is_some() {
		return Err(ClientIpError);
	}

	let value = value.to_str().map_err(|_| ClientIpError)?;

	// One canonical IP, not a chain: the edge sets exactly one address,
	// and a chain would make the choice of element a trust decision.
	if value.contains(',') {
		return Err(ClientIpError);
	}

	value.trim().parse().map(Some).map_err(|_| ClientIpError)
}

/// What a mint did with the pass.
#[derive(Debug)]
pub(crate) enum MintOutcome {
	/// The pass already carried the persona (or gained it): the cookie
	/// stays exactly as it is.
	Reused,
	/// A fresh pass was minted: the caller sets its cookie.
	Fresh {
		/// The opaque token, shown once, to bake into the cookie.
		token: String,
	},
}

/// The visitor store: one site generation's passes and people.
#[derive(Clone)]
pub(crate) struct Visitors {
	db: Db,
	/// The salt the IP hashes are keyed with, derived from the daily
	/// site key, so a pass's IP hash is useless across days.
	ip_key: [u8; 32],
	/// The generation's UTC day: what a cookie must name to be valid.
	day: String,
	/// The cookie this origin's kind uses.
	cookie: PassCookie,
	caps: Caps,
}

impl Visitors {
	/// Builds the store for one generation.
	///
	/// # Errors
	///
	/// When the daily site key can't derive the IP-hash salt.
	pub(crate) fn new(db: Db, keys: &Keys, day: &str, origin: &str, caps: Caps) -> Result<Self> {
		let ip_key = keys
			.derive(&crate::generation::site_key_id(day), IP_KEY_PURPOSE)
			.context("the daily site key is missing, so visitor IPs cannot be salted")?;

		Ok(Self {
			db,
			ip_key,
			day: day.to_owned(),
			cookie: PassCookie::of_origin(origin),
			caps,
		})
	}

	/// The cookie this origin's kind uses.
	#[must_use]
	pub(crate) const fn cookie(&self) -> PassCookie {
		self.cookie
	}

	/// The generation's UTC day.
	#[must_use]
	pub(crate) fn day(&self) -> &str {
		&self.day
	}

	/// Resolves a request's pass cookie against this generation's
	/// database. A cookie that is absent, malformed, or unknown is
	/// [`PassState::Absent`]; one that names an earlier day is
	/// [`PassState::EarlierDay`]. The caller clears both.
	pub(crate) async fn resolve(&self, headers: &HeaderMap) -> Result<PassState> {
		let Some(raw) = carried_cookie(headers, self.cookie.name) else {
			return Ok(PassState::Absent);
		};

		let Some((cookie_day, token)) = raw.split_once('.') else {
			return Ok(PassState::Absent);
		};

		if cookie_day != self.day {
			return Ok(PassState::EarlierDay);
		}

		if !is_token_shape(token) {
			return Ok(PassState::Absent);
		}

		let db = self.db.system("demo: resolve a visitor pass");
		let pass = DemoPass::query()
			.filter(|pass| pass.token_hash.eq(token_hash(token)))
			.first(&db)
			.await
			.context("looking up the visitor pass")?;

		Ok(pass.map_or(PassState::Absent, PassState::Valid))
	}

	/// Mints — or reuses — the pass and persona a request asks for.
	///
	/// With `existing`, the pass keeps its cookie and gains the persona
	/// it is missing: at most two people, one each, never against the
	/// caps. Without one, a fresh pass is minted under both caps,
	/// counted and inserted in one immediate transaction, so concurrent
	/// requests and two service processes on one volume cannot race
	/// past them.
	///
	/// # Errors
	///
	/// [`MintError::IpLimited`] and [`MintError::Full`] for the caps,
	/// [`MintError::Database`] when the database fails — an explicit
	/// failure that never mints.
	pub(crate) async fn mint(
		&self,
		persona: Persona,
		ip: Option<IpAddr>,
		existing: Option<&DemoPass>,
	) -> Result<MintOutcome, MintError> {
		let db = self.db.system("demo: mint a visitor pass");
		let caps = self.caps;
		let ip_key = self.ip_key;

		db.transaction(async |tx| {
			if let Some(pass) = existing {
				return add_person(tx, pass, persona).await;
			}

			let ip_hash = ip.map(|ip| salt_ip(ip_key, ip));

			if let Some(ip_hash) = &ip_hash {
				let window_start = leptoskit_db::now() - IP_WINDOW;
				let recent = DemoPass::query()
					.filter(|pass| {
						pass.ip_hash.eq(ip_hash.clone()) & pass.created_at.gt(window_start)
					})
					.count(tx)
					.await?;

				if recent >= caps.new_passes_per_ip_per_hour {
					let oldest = DemoPass::query()
						.filter(|pass| {
							pass.ip_hash.eq(ip_hash.clone()) & pass.created_at.gt(window_start)
						})
						.order_by(|pass| pass.created_at.asc())
						.first(tx)
						.await?;
					let one_second =
						chrono::TimeDelta::try_seconds(1).expect("one second is representable");
					let retry_after = oldest.map_or(IP_WINDOW, |oldest| {
						(oldest.created_at + IP_WINDOW - leptoskit_db::now()).max(one_second)
					});

					return Err(MintError::IpLimited { retry_after });
				}
			}

			let total = DemoPass::query().count(tx).await?;

			if total >= caps.new_passes_per_day {
				return Err(MintError::Full);
			}

			let token = random_token()?;
			let pass = NewDemoPass {
				token_hash: token_hash(&token),
				ip_hash,
			}
			.insert(tx)
			.await?;

			let minted = minted_email(persona)?;
			NewDemoPerson {
				pass_id: pass.id,
				persona,
				email: minted,
				account_id: None,
				last_receipt_id: None,
				last_receipt_at: None,
			}
			.insert(tx)
			.await?;

			Ok(MintOutcome::Fresh { token })
		})
		.await
	}

	/// Whether `email` names one of `pass`'s people — the sign-in
	/// gate's only ownership question. An address that is not even a
	/// valid address owns nothing.
	pub(crate) async fn owns(&self, pass: &DemoPass, email: &str) -> Result<bool> {
		let Ok(email) = Email::parse(email) else {
			return Ok(false);
		};

		let db = self.db.system("demo: check pass ownership");
		let owned = DemoPerson::query()
			.filter(|person| person.pass_id.eq(pass.id) & person.email.eq(email))
			.first(&db)
			.await
			.context("checking pass ownership")?;

		Ok(owned.is_some())
	}

	/// The pass's people, at most two.
	pub(crate) async fn people(&self, pass: &DemoPass) -> Result<Vec<DemoPerson>> {
		let db = self.db.system("demo: read a pass's people");
		let people = DemoPerson::query()
			.filter(|person| person.pass_id.eq(pass.id))
			.all(&db)
			.await
			.context("reading the pass's people")?;

		Ok(people)
	}
}

/// Adds `persona` to a pass inside its transaction: reuse when the pass
/// already owns the persona, an insert when it has room, and an
/// explicit error when it is somehow full — which no two-persona
/// request can reach.
async fn add_person(tx: &Tx, pass: &DemoPass, persona: Persona) -> Result<MintOutcome, MintError> {
	let existing = DemoPerson::query()
		.filter(|person| person.pass_id.eq(pass.id) & person.persona.eq(persona))
		.first(tx)
		.await?;

	if existing.is_some() {
		return Ok(MintOutcome::Reused);
	}

	let owned = DemoPerson::query()
		.filter(|person| person.pass_id.eq(pass.id))
		.count(tx)
		.await?;

	if owned >= 2 {
		return Err(MintError::Database(anyhow::anyhow!(
			"a pass owns at most two people; this one owns {owned}"
		)));
	}

	let minted = minted_email(persona)?;
	NewDemoPerson {
		pass_id: pass.id,
		persona,
		email: minted,
		account_id: None,
		last_receipt_id: None,
		last_receipt_at: None,
	}
	.insert(tx)
	.await?;

	Ok(MintOutcome::Reused)
}

/// Whether `token` is the exact shape this server mints: 64 lowercase
/// hex characters. An uppercase or shortened cookie is simply not a
/// token this server issued.
fn is_token_shape(token: &str) -> bool {
	token.len() == TOKEN_BYTES * 2
		&& token
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The stored form of a pass token: its SHA-256 over the domain
/// separator.
fn token_hash(token: &str) -> String {
	let mut hasher = Sha256::new();
	hasher.update(TOKEN_DOMAIN);
	hasher.update([0]);
	hasher.update(token.as_bytes());
	hex::encode(hasher.finalize())
}

/// The stored form of a client IP: its SHA-256 under the day's salt.
/// No address is ever stored.
fn salt_ip(ip_key: [u8; 32], ip: IpAddr) -> String {
	let mut hasher = Sha256::new();
	hasher.update(ip_key);
	hasher.update([0]);
	hasher.update(ip.to_string().as_bytes());
	hex::encode(hasher.finalize())
}

/// A fresh pass token: 32 random bytes, hex-encoded.
fn random_token() -> Result<String> {
	let mut bytes = [0_u8; TOKEN_BYTES];
	getrandom::fill(&mut bytes).context("the OS random source failed")?;
	Ok(hex::encode(bytes))
}

/// The synthetic address a person is minted with:
/// `<persona>-<16 random hex characters>@demo.invalid`.
fn minted_email(persona: Persona) -> Result<Email> {
	let mut bytes = [0_u8; EMAIL_RANDOM_CHARS / 2];
	getrandom::fill(&mut bytes).context("the OS random source failed")?;
	let random = hex::encode(bytes);
	Email::parse(format!("{}-{random}@demo.invalid", persona.slug()))
		.context("a minted address is always a valid address")
}

/// The display tag an address spells: the first four characters of its
/// random part, uppercased.
#[must_use]
pub(crate) fn display_tag(email: &Email) -> String {
	let local = email.as_inner().split('@').next().unwrap_or_default();
	let random = local.split_once('-').map_or(local, |(_, random)| random);
	random
		.chars()
		.take(4)
		.collect::<String>()
		.to_ascii_uppercase()
}

/// The `Location` target a minted or reused pass sends the visitor to:
/// local sign-in, with the persona and a validated `return_to`.
#[must_use]
pub(crate) fn sign_in_location(persona: Persona, return_to: &str) -> String {
	format!(
		"/sign-in?as={persona}&return_to={encoded}",
		persona = persona.slug(),
		encoded = return_to::encoded_query_value(return_to)
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn personas_parse_from_form_values_only() {
		assert_eq!(Persona::of_form("ada"), Some(Persona::Ada));
		assert_eq!(Persona::of_form(" grace "), Some(Persona::Grace));
		assert_eq!(Persona::of_form("ADA"), None, "the form is lowercase");
		assert_eq!(Persona::of_form("admin"), None);
		assert_eq!(Persona::of_form(""), None);
	}

	#[test]
	fn cookie_names_follow_the_origin_kind() {
		let https = PassCookie::of_origin("https://demo.example");
		assert_eq!(https.name, "__Host-demo-pass");
		assert!(https.secure);

		let plain = PassCookie::of_origin("http://localhost");
		assert_eq!(plain.name, "demo-pass");
		assert!(!plain.secure);
	}

	#[test]
	fn set_cookies_carry_the_flags_and_never_a_domain() {
		let cookie = PassCookie::of_origin("https://demo.example");
		let value = cookie
			.set("2026-11-15.abc", 3600)
			.to_str()
			.expect("header")
			.to_owned();
		assert!(
			value.starts_with("__Host-demo-pass=2026-11-15.abc;"),
			"{value}"
		);
		assert!(value.contains("Path=/"));
		assert!(value.contains("HttpOnly"));
		assert!(value.contains("SameSite=Lax"));
		assert!(value.contains("Secure"));
		assert!(value.contains("Max-Age=3600"));
		assert!(!value.to_ascii_lowercase().contains("domain"));

		let plain = PassCookie::of_origin("http://localhost");
		let plain_value = plain.set("2026-11-15.abc", 60);
		let value = plain_value.to_str().expect("header");
		assert!(value.starts_with("demo-pass="), "{value}");
		assert!(!value.contains("Secure"), "plain HTTP is not Secure");
	}

	#[test]
	fn a_cleared_cookie_is_empty_with_max_age_zero() {
		let cookie = PassCookie::of_origin("https://demo.example");
		let value = cookie.clear().to_str().expect("header").to_owned();
		assert!(value.contains("Max-Age=0"), "{value}");
		assert!(value.contains("=;"), "no value survives: {value}");
	}

	#[test]
	fn midnight_is_bounded_by_one_day() {
		let noon = chrono::DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
			.expect("a timestamp")
			.with_timezone(&chrono::Utc);
		assert_eq!(seconds_until_next_midnight(noon), 12 * 3600);

		let almost = chrono::DateTime::parse_from_rfc3339("2026-11-15T23:59:59Z")
			.expect("a timestamp")
			.with_timezone(&chrono::Utc);
		assert_eq!(seconds_until_next_midnight(almost), 1);
	}

	#[test]
	fn minted_addresses_are_persona_shaped_with_a_display_tag() {
		let email = minted_email(Persona::Ada).expect("mints");
		let text = email.as_inner().to_owned();
		let (local, domain) = text.split_once('@').expect("an @");
		assert_eq!(domain, "demo.invalid");
		let (persona, random) = local.split_once('-').expect("a dash");
		assert_eq!(persona, "ada");
		assert_eq!(random.len(), EMAIL_RANDOM_CHARS);
		assert!(random.bytes().all(|byte| byte.is_ascii_hexdigit()));
		assert_eq!(display_tag(&email), random[..4].to_ascii_uppercase());

		let grace = minted_email(Persona::Grace).expect("mints");
		assert!(grace.as_inner().starts_with("grace-"));
		assert_ne!(email, grace, "two mints differ");

		// The space is random: every mint in a run is its own address.
		let mut seen = std::collections::HashSet::new();
		for _ in 0..256 {
			seen.insert(minted_email(Persona::Ada).expect("mints"));
		}
		assert_eq!(seen.len(), 256, "every mint is its own address");
	}

	#[test]
	fn token_hashes_domain_separate_the_token() {
		let hashed = token_hash("2026-11-15.abc");
		assert_eq!(hashed.len(), 64);
		assert_ne!(token_hash("2026-11-15.abc"), token_hash("2026-11-16.abc"));
		assert_eq!(token_hash("2026-11-15.abc"), token_hash("2026-11-15.abc"));

		let token = random_token().expect("random");
		assert_eq!(token.len(), TOKEN_BYTES * 2);
		assert!(is_token_shape(&token));
		assert!(!is_token_shape(&token.to_ascii_uppercase()));
		assert!(!is_token_shape("short"));
	}

	#[test]
	fn client_ip_trusts_only_one_place() {
		let headers = |values: &[(&str, &str)]| {
			let mut headers = HeaderMap::new();
			for (name, value) in values {
				headers.insert(
					header::HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
					header::HeaderValue::from_str(value).expect("a header value"),
				);
			}
			headers
		};
		let peer: SocketAddr = "127.0.0.1:40000".parse().expect("a peer");

		// No header configured: only the peer is trusted, and a
		// forwarding header is never read.
		let forwarded = headers(&[("x-forwarded-for", "203.0.113.9")]);
		assert_eq!(
			client_ip(None, &forwarded, Some(peer)).expect("peer is fine"),
			Some("127.0.0.1".parse().expect("an ip"))
		);
		assert_eq!(client_ip(None, &forwarded, None).expect("no peer"), None);

		// Configured: exactly one value that is exactly one address.
		let good = headers(&[("x-forwarded-for", "203.0.113.9")]);
		assert_eq!(
			client_ip(Some("x-forwarded-for"), &good, Some(peer)).expect("a single address"),
			Some("203.0.113.9".parse().expect("an ip"))
		);

		// A comma chain is refused, not first-picked.
		let chained = headers(&[("x-forwarded-for", "203.0.113.9, 198.51.100.7")]);
		assert!(client_ip(Some("x-forwarded-for"), &chained, Some(peer)).is_err());

		// A duplicated header is refused.
		let mut duplicated = HeaderMap::new();
		duplicated.append(
			header::HeaderName::from_static("x-forwarded-for"),
			header::HeaderValue::from_static("203.0.113.9"),
		);
		duplicated.append(
			header::HeaderName::from_static("x-forwarded-for"),
			header::HeaderValue::from_static("203.0.113.8"),
		);
		assert!(client_ip(Some("x-forwarded-for"), &duplicated, Some(peer)).is_err());

		// A non-address is refused, never treated as absent.
		let garbage = headers(&[("x-forwarded-for", "not-an-ip")]);
		assert!(client_ip(Some("x-forwarded-for"), &garbage, Some(peer)).is_err());

		// Absent header: no IP, the same answer the peer-less local
		// deployment gives.
		assert_eq!(
			client_ip(Some("x-forwarded-for"), &headers(&[]), Some(peer))
				.expect("an absent header is no ip"),
			None
		);
	}

	#[test]
	fn the_sign_in_location_carries_the_persona_and_encoded_return() {
		let location = sign_in_location(Persona::Ada, "/notes");
		assert_eq!(location, "/sign-in?as=ada&return_to=%2Fnotes");

		let location = sign_in_location(Persona::Grace, "/oauth/authorize?client_id=abc");
		assert_eq!(
			location,
			"/sign-in?as=grace&return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dabc"
		);
	}
}
