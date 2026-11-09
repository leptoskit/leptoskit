//! Capture exact dual-stack origin ranges before rendering a provider plan.

use std::collections::BTreeSet;
use std::net::IpAddr;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use serde_json::Value;
use serde_json::json;
use sha2::Digest as _;

const IPV4_SOURCE: &str = "https://www.cloudflare.com/ips-v4";
const IPV6_SOURCE: &str = "https://www.cloudflare.com/ips-v6";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_RANGES: usize = 1024;

pub(super) struct Ranges {
	ipv4: Vec<String>,
	ipv6: Vec<String>,
}

impl Ranges {
	pub(super) fn fetch() -> Result<Self> {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(std::time::Duration::from_secs(30)))
			.build()
			.into();
		let read = |url: &str| -> Result<String> {
			let mut response = agent.get(url).call().context(
				"could not fetch Cloudflare origin ranges; no infrastructure files were written",
			)?;
			let body = response
				.body_mut()
				.with_config()
				.limit(MAX_RESPONSE_BYTES as u64)
				.read_to_string()?;
			ensure!(
				body.len() <= MAX_RESPONSE_BYTES,
				"Cloudflare range response exceeds its size limit"
			);
			Ok(body)
		};
		Self::parse(&read(IPV4_SOURCE)?, &read(IPV6_SOURCE)?)
	}

	pub(super) fn parse(ipv4: &str, ipv6: &str) -> Result<Self> {
		Ok(Self {
			ipv4: parse(ipv4, true)?,
			ipv6: parse(ipv6, false)?,
		})
	}

	/// Read the canonical snapshot retained by generation, without refreshing
	/// it.
	pub(super) fn from_manifest(value: &Value) -> Result<Self> {
		let text = |key: &str| -> Result<String> {
			let entries = value[key]
				.as_array()
				.context("origin range snapshot lacks a list")?;
			let ranges = entries
				.iter()
				.map(|entry| entry.as_str().context("invalid retained origin range"))
				.collect::<Result<Vec<_>>>()?;
			Ok(ranges.join("\n"))
		};
		let ranges = Self::parse(&text("ipv4")?, &text("ipv6")?)?;
		ensure!(
			ranges.manifest() == *value,
			"origin range snapshot is not canonical or its digest/sources changed"
		);
		Ok(ranges)
	}

	pub(super) fn all(&self) -> Vec<&str> {
		self.ipv4
			.iter()
			.chain(&self.ipv6)
			.map(String::as_str)
			.collect()
	}

	pub(super) fn manifest(&self) -> Value {
		// Canonical lists bind the actual rules, independently of response
		// whitespace or the order in which the provider publishes its ranges.
		let canonical = format!("{}\n{}\n", self.ipv4.join("\n"), self.ipv6.join("\n"));
		json!({"ipv4_source": IPV4_SOURCE, "ipv6_source": IPV6_SOURCE,
			"ipv4": self.ipv4, "ipv6": self.ipv6,
			"sha256": hex::encode(sha2::Sha256::digest(canonical.as_bytes()))})
	}
}

fn parse(body: &str, ipv4: bool) -> Result<Vec<String>> {
	ensure!(
		body.len() <= MAX_RESPONSE_BYTES,
		"origin range response exceeds its size limit"
	);
	let mut ranges = BTreeSet::new();

	for line in body.lines() {
		let value = line.trim();
		let (address, prefix) = value
			.split_once('/')
			.context("origin range is not a CIDR")?;
		let address: IpAddr = address
			.parse()
			.context("origin range has an invalid address")?;
		let prefix: u8 = prefix
			.parse()
			.context("origin range has an invalid prefix")?;
		ensure!(
			address.is_ipv4() == ipv4,
			"origin range has the wrong address family"
		);
		ensure!(
			valid_network(address, prefix),
			"origin range is broad, local or noncanonical"
		);
		ensure!(
			ranges.insert(format!("{address}/{prefix}")),
			"duplicate origin range"
		);

		ensure!(ranges.len() <= MAX_RANGES, "too many origin ranges");
	}

	ensure!(
		!ranges.is_empty(),
		"both IPv4 and IPv6 origin ranges are required"
	);
	Ok(ranges.into_iter().collect())
}

fn valid_network(address: IpAddr, prefix: u8) -> bool {
	if address.is_unspecified() || address.is_loopback() || address.is_multicast() {
		return false;
	}

	match address {
		IpAddr::V4(address) => {
			(8..=32).contains(&prefix)
				&& !address.is_private()
				&& !address.is_link_local()
				&& u32::from(address).trailing_zeros() >= u32::BITS - u32::from(prefix)
		}
		IpAddr::V6(address) => {
			(16..=128).contains(&prefix)
				&& !address.is_unique_local()
				&& !address.is_unicast_link_local()
				&& u128::from(address).trailing_zeros() >= u128::BITS - u32::from(prefix)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ranges_bind_both_families_without_open_fallback() {
		let ranges =
			Ranges::parse("173.245.48.0/20\n103.21.244.0/22\n", "2400:cb00::/32\n").unwrap();
		assert_eq!(
			ranges.all(),
			["103.21.244.0/22", "173.245.48.0/20", "2400:cb00::/32"]
		);
		let reordered =
			Ranges::parse("103.21.244.0/22\n173.245.48.0/20\n", "2400:cb00::/32").unwrap();
		assert_eq!(ranges.manifest(), reordered.manifest());
	}

	#[test]
	fn unknown_empty_broad_and_mixed_responses_fail() {
		for invalid in [
			"",
			"<html>error</html>",
			"0.0.0.0/0",
			"128.0.0.0/1",
			"127.0.0.0/8",
			"10.0.0.0/8",
			"173.245.48.1/20",
			"2400:cb00::/32",
			"173.245.48.0/20\n173.245.48.0/20",
		] {
			assert!(
				Ranges::parse(invalid, "2400:cb00::/32").is_err(),
				"{invalid}"
			);
		}
		for invalid in [
			"",
			"::/0",
			"::1/128",
			"fc00::/7",
			"fe80::/64",
			"2400:cb00::1/32",
			"173.245.48.0/20",
		] {
			assert!(
				Ranges::parse("173.245.48.0/20", invalid).is_err(),
				"{invalid}"
			);
		}
	}
}
