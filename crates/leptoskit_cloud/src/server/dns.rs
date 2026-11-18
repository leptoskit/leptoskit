//! Reading a domain's challenge record.
//!
//! The control plane asks one question of DNS: the TXT records at a name.
//! It asks a resolver over HTTPS ([`DnsOverHttps`], the JSON form that
//! [Cloudflare](https://developers.cloudflare.com/1.1.1.1/encryption/dns-over-https/make-api-requests/dns-json/)
//! and [Google](https://developers.google.com/speed/public-dns/docs/doh/json)
//! serve), so the answer doesn't depend on the machine's own resolver. Tests
//! put their own [`Resolver`] in its place.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

/// How long one lookup may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// The most of an answer that is read.
const MAX_ANSWER_LEN: usize = 64 * 1024;

/// The most records taken from one answer.
pub(crate) const MAX_RECORDS: usize = 32;

/// The most of one record that is kept.
pub(crate) const MAX_RECORD_LEN: usize = 512;

/// The DNS record type for text.
const TXT: u16 = 16;

/// The name doesn't exist. Not an error: it has no records.
const NXDOMAIN: u16 = 3;

/// Why a lookup gave no answer. Never the reason in detail: it would name
/// the resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the DNS lookup failed")]
pub struct ResolveError;

/// What a lookup answers with.
pub type Lookup<'a> = Pin<Box<dyn Future<Output = Result<Vec<String>, ResolveError>> + Send + 'a>>;

/// Something that reads TXT records.
pub trait Resolver: Send + Sync + std::fmt::Debug {
	/// The TXT records at `name`, each with its strings joined. A name
	/// that doesn't exist has none.
	fn txt<'a>(&'a self, name: &'a str) -> Lookup<'a>;
}

/// A [`Resolver`] the control plane shares.
pub type SharedResolver = Arc<dyn Resolver>;

/// A resolver reached over HTTPS, answering in JSON.
#[derive(Clone, Debug)]
pub struct DnsOverHttps {
	endpoint: reqwest::Url,
	client: reqwest::Client,
}

#[derive(Deserialize)]
struct Answer {
	#[serde(rename = "Status")]
	status: u16,
	#[serde(rename = "Answer", default)]
	answer: Vec<Record>,
}

#[derive(Deserialize)]
struct Record {
	#[serde(rename = "type")]
	kind: u16,
	#[serde(default)]
	data: String,
}

impl DnsOverHttps {
	/// A resolver at `endpoint`, such as
	/// `https://cloudflare-dns.com/dns-query`.
	///
	/// # Errors
	///
	/// When `endpoint` isn't an `http` or `https` address, or has a query
	/// of its own.
	pub fn new(endpoint: &str) -> Result<Self, String> {
		let endpoint = reqwest::Url::parse(endpoint)
			.map_err(|_| "the DNS resolver's address isn't a URL".to_owned())?;

		if !matches!(endpoint.scheme(), "https" | "http")
			|| endpoint.query().is_some()
			|| endpoint.fragment().is_some()
			|| !endpoint.username().is_empty()
			|| endpoint.password().is_some()
		{
			return Err(
				"the DNS resolver's address must be http(s), with no query, fragment, or \
				 credentials"
					.to_owned(),
			);
		}

		let client = reqwest::Client::builder()
			.timeout(LOOKUP_TIMEOUT)
			.redirect(reqwest::redirect::Policy::none())
			.build()
			.map_err(|_| "can't make an HTTP client".to_owned())?;
		Ok(Self { endpoint, client })
	}

	async fn lookup(&self, name: &str) -> Result<Vec<String>, ResolveError> {
		let mut address = self.endpoint.clone();
		address
			.query_pairs_mut()
			.append_pair("name", name)
			.append_pair("type", "TXT");
		let mut response = self
			.client
			.get(address)
			.header("accept", "application/dns-json")
			.send()
			.await
			.map_err(|_| ResolveError)?;

		if !response.status().is_success() {
			return Err(ResolveError);
		}

		let mut body = Vec::new();

		while let Some(chunk) = response.chunk().await.map_err(|_| ResolveError)? {
			if body.len() + chunk.len() > MAX_ANSWER_LEN {
				return Err(ResolveError);
			}

			body.extend_from_slice(&chunk);
		}

		parse(&body)
	}
}

impl Resolver for DnsOverHttps {
	fn txt<'a>(&'a self, name: &'a str) -> Lookup<'a> {
		Box::pin(self.lookup(name))
	}
}

/// The TXT records in a JSON answer.
fn parse(body: &[u8]) -> Result<Vec<String>, ResolveError> {
	let answer: Answer = serde_json::from_slice(body).map_err(|_| ResolveError)?;

	if answer.status == NXDOMAIN {
		return Ok(Vec::new());
	}

	if answer.status != 0 {
		return Err(ResolveError);
	}

	Ok(answer
		.answer
		.iter()
		.filter(|record| record.kind == TXT)
		.take(MAX_RECORDS)
		.map(|record| unquote(&record.data))
		.collect())
}

/// A TXT record's strings, joined: `"ab" "cd"` is `abcd`. A resolver that
/// sends the text bare is taken as it is.
fn unquote(data: &str) -> String {
	let data = data.trim();

	if !data.starts_with('"') {
		return data.chars().take(MAX_RECORD_LEN).collect();
	}

	let mut text = String::new();
	let mut inside = false;
	let mut characters = data.chars();

	while let Some(character) = characters.next() {
		match character {
			'"' => inside = !inside,
			'\\' if inside => {
				if let Some(escaped) = characters.next() {
					text.push(escaped);
				}
			}
			_ if inside => text.push(character),
			_ => {}
		}
	}

	text.chars().take(MAX_RECORD_LEN).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn answers_are_read_as_txt_records() {
		let body = br#"{"Status":0,"Answer":[
			{"name":"_c.example.com","type":16,"TTL":300,"data":"\"leptoskit-verify=abc\""},
			{"name":"_c.example.com","type":16,"TTL":300,"data":"\"split \" \"in two\""},
			{"name":"_c.example.com","type":5,"TTL":300,"data":"elsewhere.example.net."},
			{"name":"_c.example.com","type":16,"TTL":300,"data":"bare"},
			{"name":"_c.example.com","type":16,"TTL":300,"data":"\"a \\\"quote\\\"\""}
		]}"#;
		assert_eq!(
			parse(body),
			Ok(vec![
				"leptoskit-verify=abc".to_owned(),
				"split in two".to_owned(),
				"bare".to_owned(),
				"a \"quote\"".to_owned(),
			])
		);
	}

	#[test]
	fn a_name_that_does_not_exist_has_no_records() {
		assert_eq!(parse(br#"{"Status":3}"#), Ok(Vec::new()));
		assert_eq!(parse(br#"{"Status":0}"#), Ok(Vec::new()));
	}

	#[test]
	fn a_failed_or_unreadable_answer_is_an_error() {
		assert_eq!(parse(br#"{"Status":2}"#), Err(ResolveError));
		assert_eq!(parse(b"<html>"), Err(ResolveError));
	}

	#[test]
	fn answers_are_bounded() {
		let records: Vec<String> = (0..100)
			.map(|_| format!(r#"{{"type":16,"data":"\"{}\""}}"#, "x".repeat(2_000)))
			.collect();
		let body = format!(r#"{{"Status":0,"Answer":[{}]}}"#, records.join(","));
		let parsed = parse(body.as_bytes()).expect("parsed");
		assert_eq!(parsed.len(), MAX_RECORDS);
		assert!(parsed.iter().all(|record| record.len() == MAX_RECORD_LEN));
	}

	#[test]
	fn the_resolver_is_an_address_with_nothing_of_its_own() {
		assert!(DnsOverHttps::new("https://cloudflare-dns.com/dns-query").is_ok());

		for bad in [
			"cloudflare-dns.com",
			"ftp://example.com/dns",
			"https://example.com/dns?name=x",
			"https://user:pass@example.com/dns",
		] {
			assert!(DnsOverHttps::new(bad).is_err(), "{bad}");
		}
	}
}
