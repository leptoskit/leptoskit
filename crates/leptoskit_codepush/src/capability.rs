//! Versioned native capability contracts.
//!
//! A host descriptor is typed, deterministic, and derived from what the build
//! actually selected: the destination flavour, the generated host's plugin
//! flavours, and the app's non-secret plugin options. It never contains
//! machine paths, timestamps, secret values, or a downloaded guest's claims.
//!
//! A guest carries [`RuntimeRequirements`] instead: the capability identifiers
//! its app cannot run without, and the ones it can. These requirements sit
//! in the signed bundle (`runtime.json`); the host compares them with the
//! descriptor embedded in its store build before staging or activation.
//!
//! Declared build support is not runtime evidence. A descriptor says what a
//! host build is expected to provide; device probes, widget suites, and code
//! push journeys remain separate evidence and are never inferred from it.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;
use crate::installed::activation;
use crate::manifest::hash_bytes;

/// The descriptor format this build writes and reads.
pub const CAPABILITY_CONTRACT_VERSION: u32 = 1;

/// The signed runtime descriptor format this build writes. Version 1 was the
/// implicit `{abi, shell, plugins}` object with no capability contract.
pub const RUNTIME_CONTRACT_VERSION: u32 = 2;

/// A host-level capability: synchronous Rust plugins.
pub const CAP_HOST_PLUGIN_RUST_SYNC: &str = "host.plugin.rust.sync";
/// A host-level capability: Rust plugins on a worker or the UI thread.
pub const CAP_HOST_PLUGIN_RUST_WORKER: &str = "host.plugin.rust.worker";
/// A host-level capability: Rust plugins on the host's async runtime.
pub const CAP_HOST_PLUGIN_RUST_ASYNC: &str = "host.plugin.rust.async";
/// A host-level capability: Rust application services.
pub const CAP_HOST_PLUGIN_SERVICE: &str = "host.plugin.service";
/// A host-level capability: AppKit/UIKit plugin types.
pub const CAP_HOST_PLUGIN_NATIVE_APPLE: &str = "host.plugin.native.apple";
/// A host-level capability: Android/Kotlin plugin types.
pub const CAP_HOST_PLUGIN_NATIVE_ANDROID: &str = "host.plugin.native.android";
/// A host-level capability: HarmonyOS/ArkTS plugin types.
pub const CAP_HOST_PLUGIN_NATIVE_ARKTS: &str = "host.plugin.native.arkts";
/// A host-level capability: automatic code push checks.
pub const CAP_HOST_UPDATE_CODEPUSH: &str = "host.update.codepush";
/// A host-level capability: the generated host registers the reserved
/// `leptoskit.timer` plugin and wakes at the next deadline.
pub const CAP_HOST_TIMER: &str = "host.timer";
/// A host-level capability: the destination itself is experimental.
pub const CAP_HOST_PLATFORM_EXPERIMENTAL: &str = "host.platform.experimental";

/// The capability identifier for a granted runtime plugin.
pub fn plugin_capability(name: &str) -> String {
	format!("plugin.{name}")
}

/// Whether `value` is a stable capability identifier.
pub fn capability_identifier(value: &str) -> bool {
	let mut characters = value.chars();
	characters
		.next()
		.is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
		&& characters.all(|character| {
			character.is_ascii_lowercase()
				|| character.is_ascii_digit()
				|| matches!(character, '-' | '_' | '.')
		})
		&& !value.split('.').any(str::is_empty)
}

/// What a host build says its implementation of a capability is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
	/// The generated host runs the capability.
	Available,
	/// The generated host cannot run the capability.
	Unsupported,
	/// The generated host runs it only through an implementation the project
	/// opted into as experimental.
	Experimental,
	/// The descriptor does not describe the identifier.
	Unknown,
}

impl CapabilityState {
	/// Whether a required capability may run, and whether it is experimental.
	pub fn usable(self) -> bool {
		matches!(self, Self::Available | Self::Experimental)
	}
}

/// One capability the host descriptor describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEntry {
	/// Stable identifier, such as `host.plugin.rust.async`.
	pub id: String,
	/// What this host build can do.
	pub state: CapabilityState,
	/// Why the state is not `available`, in plain words.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub detail: Option<String>,
}

/// The plugin protocol and rendering ABI a host speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersions {
	/// The native Wasm guest ABI.
	pub guest_abi: u32,
	/// The plugin bridge protocol.
	pub bridge: u32,
}

/// One granted runtime plugin as this host build resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCapability {
	/// The plugin's stable metadata name, such as `http`.
	pub name: String,
	/// What this host build resolved for the plugin.
	pub state: CapabilityState,
	/// The implementation flavour, such as `rust.sync`, `native.apple`, or
	/// `service`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub implementation: Option<String>,
	/// The host-owned option keys, sorted. Values never appear here.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub option_keys: Vec<String>,
	/// The SHA-256 of the canonical option table, so changing a value changes
	/// the contract without publishing the value.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub options_sha256: Option<String>,
	/// Why the plugin is not `available`, in plain words.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub detail: Option<String>,
}

/// The locally authoritative host contract embedded in a store build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilityDescriptor {
	/// [`CAPABILITY_CONTRACT_VERSION`].
	pub format_version: u32,
	/// The application platform, such as `ios` or `harmonyos`.
	pub platform: String,
	/// The concrete build flavour, such as `ios-simulator` or
	/// `harmonyos-x86_64`.
	pub flavor: String,
	/// Whether the destination itself is experimental.
	pub experimental: bool,
	/// Protocol versions this host speaks.
	pub protocol: ProtocolVersions,
	/// Host-level capabilities, sorted by identifier.
	pub capabilities: Vec<CapabilityEntry>,
	/// Granted plugins as resolved for this destination, sorted by name.
	pub plugins: Vec<PluginCapability>,
}

/// The capabilities a signed guest needs, split into required and optional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequirements {
	/// [`CAPABILITY_CONTRACT_VERSION`].
	pub format_version: u32,
	/// Exact guest ABI and plugin protocol required by the compiled guest.
	pub protocol: ProtocolVersions,
	/// Capabilities the guest cannot run without.
	pub required: Vec<String>,
	/// Capabilities the guest can run without.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub optional: Vec<String>,
}

/// How a signed guest's requirements related to the installed host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractCompatibility {
	/// The guest named a versioned contract and the host descriptor satisfied
	/// every required capability and every retained legacy plugin grant.
	Verified,
	/// Explicit requirements passed, but a plugin kept under the legacy
	/// missing-plugin policy is unavailable or unknown. The release may run
	/// with its existing fallback; its full plugin compatibility is unproven.
	Incomplete,
	/// The guest predates versioned capability contracts. It passed the
	/// legacy ABI, shell, and plugin checks, but compatibility is unknown.
	#[default]
	Legacy,
}

fn invalid(message: impl Into<String>) -> CodePushError {
	CodePushError::InvalidCapabilityContract(message.into())
}

impl HostCapabilityDescriptor {
	/// Checks the version, identifiers, uniqueness, and option digests.
	pub fn validate(&self) -> Result<(), CodePushError> {
		validate_descriptor_version(self.format_version)?;
		validate_descriptor_identity(&self.platform, &self.flavor)?;
		validate_protocol(self.protocol)?;

		validate_capability_entries(&self.capabilities)?;
		let plugins = validate_plugin_entries(&self.plugins)?;

		reject_plugin_capability_conflicts(&self.capabilities, &plugins)
	}

	/// The capability or plugin state for `id`, with its detail.
	pub fn state_of(&self, id: &str) -> Option<(&CapabilityState, Option<&str>)> {
		if let Some(plugin) = id.strip_prefix("plugin.")
			&& let Some(entry) = self.plugins.iter().find(|entry| entry.name == plugin)
		{
			return Some((&entry.state, entry.detail.as_deref()));
		}

		self.capabilities
			.iter()
			.find(|entry| entry.id == id)
			.map(|entry| (&entry.state, entry.detail.as_deref()))
	}

	/// A store runtime's grant list and its descriptor must describe the same
	/// plugins. A grant permits access; the entry separately records whether
	/// the generated host has an implementation.
	pub(crate) fn validate_grants(&self, grants: &[String]) -> Result<(), CodePushError> {
		let mut names = BTreeSet::new();

		for name in grants {
			if !capability_identifier(name) || !names.insert(name.as_str()) {
				return Err(invalid(format!(
					"host plugin grant `{name}` must be a unique stable identifier"
				)));
			}
			if !self.plugins.iter().any(|plugin| plugin.name == *name) {
				return Err(invalid(format!(
					"host plugin grant `{name}` has no resolved plugin entry"
				)));
			}
		}

		for plugin in &self.plugins {
			if !names.contains(plugin.name.as_str()) {
				return Err(invalid(format!(
					"host descriptor names plugin `{}` without a plugin grant",
					plugin.name
				)));
			}
		}

		for capability in &self.capabilities {
			if capability.id.starts_with("plugin.") && capability.state.usable() {
				return Err(invalid(format!(
					"host capability `{}` needs a resolved plugin entry and grant",
					capability.id
				)));
			}
		}
		Ok(())
	}

	/// Canonical bytes for [`Self::digest`]: sorted entries and no map order.
	pub fn canonical_bytes(&self) -> Vec<u8> {
		let mut normalized = self.clone();
		normalized
			.capabilities
			.sort_by(|left, right| left.id.cmp(&right.id));
		normalized
			.plugins
			.sort_by(|left, right| left.name.cmp(&right.name));

		for plugin in &mut normalized.plugins {
			plugin.option_keys.sort();
			plugin.option_keys.dedup();
		}

		let value = serde_json::to_value(&normalized).expect("a descriptor serializes");
		let mut out = Vec::new();
		canonical_json(&value, &mut out);
		out
	}

	/// The stable SHA-256 of [`Self::canonical_bytes`].
	pub fn digest(&self) -> String {
		hash_bytes(&self.canonical_bytes())
	}
}

/// Native requirements in the bundle's `runtime.json`, or the host descriptor
/// in the store build's `codepush.json`.
///
/// A legacy descriptor has no `format_version`: it carried only `abi`,
/// `shell`, and `plugins`, so its compatibility is unknown rather than
/// verified. Version 2 additionally carries signed guest requirements in a
/// bundle, or the locally authoritative host descriptor in the store build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContract {
	/// Native guest ABI revision.
	pub abi: u32,
	/// Shell compatibility identity, changed when native capabilities change.
	pub shell: String,
	/// Plugin grants retained for the legacy compatibility check. A grant is
	/// permission to call a plugin, not evidence of a host implementation.
	/// Versioned requirements separately declare mandatory availability.
	pub plugins: Vec<String>,
	/// `None` for a legacy descriptor; otherwise [`RUNTIME_CONTRACT_VERSION`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub format_version: Option<u32>,
	/// The signed guest's required and optional capabilities.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub requirements: Option<RuntimeRequirements>,
	/// The store build's locally authoritative host contract.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub host: Option<HostCapabilityDescriptor>,
}

impl RuntimeContract {
	/// Checks a store build's descriptor. A host configuration never carries
	/// guest requirements, and a versioned one always carries its descriptor.
	pub fn validate_host(&self) -> Result<(), CodePushError> {
		match self.format_version {
			None => {
				if self.requirements.is_some() || self.host.is_some() {
					return Err(CodePushError::InvalidCapabilityContract(
						"an unversioned host runtime cannot carry contract fields".into(),
					));
				}
			}
			Some(RUNTIME_CONTRACT_VERSION) => {
				if self.requirements.is_some() {
					return Err(CodePushError::InvalidCapabilityContract(
						"a host runtime cannot carry guest requirements".into(),
					));
				}

				let host = self.host.as_ref().ok_or_else(|| {
					CodePushError::InvalidCapabilityContract(
						"a versioned host runtime needs its host capability descriptor".into(),
					)
				})?;
				host.validate()?;
				host.validate_grants(&self.plugins)?;

				if host.protocol.guest_abi != self.abi {
					return Err(CodePushError::InvalidCapabilityContract(format!(
						"host descriptor speaks guest ABI {}, but the runtime names {}",
						host.protocol.guest_abi, self.abi
					)));
				}
			}
			Some(version) => return Err(CodePushError::UnsupportedContractVersion(version)),
		}

		Ok(())
	}

	/// Checks a signed guest's descriptor against the store build's host
	/// configuration. Downloaded code never supplies the host descriptor.
	pub fn verify_guest(&self, host: &Self) -> Result<ContractCompatibility, CodePushError> {
		host.validate_host()?;
		if self.abi != host.abi
			|| self.shell != host.shell
			|| self
				.plugins
				.iter()
				.any(|plugin| !host.plugins.contains(plugin))
		{
			return Err(activation(
				"bundle requires an incompatible native ABI, shell, or plugin",
			));
		}

		match self.format_version {
			None => {
				if self.requirements.is_some() || self.host.is_some() {
					return Err(CodePushError::InvalidCapabilityContract(
						"an unversioned guest runtime cannot carry contract fields".into(),
					));
				}

				Ok(ContractCompatibility::Legacy)
			}
			Some(RUNTIME_CONTRACT_VERSION) => {
				if self.host.is_some() {
					return Err(CodePushError::InvalidCapabilityContract(
						"a downloaded guest cannot declare host capabilities".into(),
					));
				}

				let requirements = self.requirements.as_ref().ok_or_else(|| {
					CodePushError::InvalidCapabilityContract(
						"a versioned guest runtime needs its capability requirements".into(),
					)
				})?;
				let descriptor = host.host.as_ref().ok_or_else(|| {
					CodePushError::InvalidCapabilityContract(
						"the installed host has no capability descriptor; ship a store build that \
						 writes one"
							.into(),
					)
				})?;
				requirements.verify_against(descriptor)?;
				let complete = self.plugins.iter().all(|name| {
					descriptor
						.state_of(&plugin_capability(name))
						.is_some_and(|(state, _)| state.usable())
				});
				Ok(if complete {
					ContractCompatibility::Verified
				} else {
					ContractCompatibility::Incomplete
				})
			}
			Some(version) => Err(CodePushError::UnsupportedContractVersion(version)),
		}
	}
}

impl RuntimeRequirements {
	/// Checks the version and that required and optional entries are stable,
	/// unique, and disjoint.
	pub fn validate(&self) -> Result<(), CodePushError> {
		if self.format_version != CAPABILITY_CONTRACT_VERSION {
			return Err(CodePushError::UnsupportedContractVersion(
				self.format_version,
			));
		}

		if self.protocol.guest_abi == 0 || self.protocol.bridge == 0 {
			return Err(invalid("required protocol versions must be positive"));
		}

		for (name, entries) in [("required", &self.required), ("optional", &self.optional)] {
			let mut seen = BTreeSet::new();

			for id in entries {
				if !capability_identifier(id) {
					return Err(invalid(format!(
						"{name} capability `{id}` is not a stable identifier"
					)));
				}

				if !seen.insert(id.as_str()) {
					return Err(invalid(format!(
						"{name} capability `{id}` is declared more than once"
					)));
				}
			}
		}

		if let Some(id) = self.required.iter().find(|id| self.optional.contains(id)) {
			return Err(invalid(format!(
				"capability `{id}` is both required and optional"
			)));
		}

		Ok(())
	}

	/// Checks every required capability against the locally authoritative host
	/// descriptor. Optional omissions never block an update.
	pub fn verify_against(&self, host: &HostCapabilityDescriptor) -> Result<(), CodePushError> {
		self.validate()?;
		host.validate()?;

		if self.protocol != host.protocol {
			return Err(invalid(format!(
				"guest requires protocols {:?}, but the installed host provides {:?}",
				self.protocol, host.protocol
			)));
		}

		for id in &self.required {
			let Some((state, detail)) = host.state_of(id) else {
				return Err(invalid(format!(
					"guest requires `{id}`, which the installed host contract does not describe"
				)));
			};

			if !state.usable() {
				return Err(invalid(format!(
					"guest requires `{id}`, but the installed host reports it {state:?}{}",
					detail.map_or_else(String::new, |detail| format!(": {detail}"))
				)));
			}
		}

		Ok(())
	}
}

/// A SHA-256 digest is written as 64 lowercase hexadecimal characters.
const SHA256_HEX_LENGTH: usize = 64;

fn validate_descriptor_version(version: u32) -> Result<(), CodePushError> {
	if version == CAPABILITY_CONTRACT_VERSION {
		Ok(())
	} else {
		Err(CodePushError::UnsupportedContractVersion(version))
	}
}

fn validate_descriptor_identity(platform: &str, flavor: &str) -> Result<(), CodePushError> {
	if capability_identifier(platform) && capability_identifier(flavor) {
		Ok(())
	} else {
		Err(invalid("platform and flavor must be stable identifiers"))
	}
}

fn validate_protocol(protocol: ProtocolVersions) -> Result<(), CodePushError> {
	if protocol.guest_abi == 0 || protocol.bridge == 0 {
		Err(invalid("protocol versions must be positive"))
	} else {
		Ok(())
	}
}

fn validate_capability_entries(entries: &[CapabilityEntry]) -> Result<(), CodePushError> {
	let mut identifiers = BTreeSet::new();

	for capability in entries {
		if !capability_identifier(&capability.id) {
			return Err(invalid(format!(
				"`{}` is not a stable capability identifier",
				capability.id
			)));
		}

		if !identifiers.insert(capability.id.as_str()) {
			return Err(invalid(format!(
				"capability `{}` is declared more than once",
				capability.id
			)));
		}
	}

	Ok(())
}

fn validate_plugin_entries(entries: &[PluginCapability]) -> Result<BTreeSet<&str>, CodePushError> {
	let mut names = BTreeSet::new();

	for plugin in entries {
		if !capability_identifier(&plugin.name) {
			return Err(invalid(format!(
				"plugin `{}` is not a stable identifier",
				plugin.name
			)));
		}

		if !names.insert(plugin.name.as_str()) {
			return Err(invalid(format!(
				"plugin `{}` is declared more than once",
				plugin.name
			)));
		}

		if plugin.option_keys.windows(2).any(|pair| pair[0] >= pair[1]) {
			return Err(invalid(format!(
				"plugin `{}` option keys must be sorted and unique",
				plugin.name
			)));
		}

		if plugin
			.options_sha256
			.as_deref()
			.is_some_and(|digest| !is_sha256(digest))
		{
			return Err(invalid(format!(
				"plugin `{}` options digest must be lowercase hex SHA-256",
				plugin.name
			)));
		}
	}

	Ok(names)
}

fn reject_plugin_capability_conflicts(
	capabilities: &[CapabilityEntry],
	plugins: &BTreeSet<&str>,
) -> Result<(), CodePushError> {
	for capability in capabilities {
		if let Some(name) = capability.id.strip_prefix("plugin.")
			&& plugins.contains(name)
		{
			return Err(invalid(format!(
				"capability `{}` conflicts with the plugin entry of the same name",
				capability.id
			)));
		}
	}

	Ok(())
}

fn is_sha256(value: &str) -> bool {
	value.len() == SHA256_HEX_LENGTH
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Canonical JSON with sorted object keys, so map iteration order can never
/// change a digest.
fn canonical_json(value: &serde_json::Value, out: &mut Vec<u8>) {
	match value {
		serde_json::Value::Object(map) => canonical_object(map, out),
		serde_json::Value::Array(values) => canonical_array(values, out),
		other => out.extend(serde_json::to_vec(other).expect("a JSON value serializes")),
	}
}

fn canonical_object(map: &serde_json::Map<String, serde_json::Value>, out: &mut Vec<u8>) {
	out.push(b'{');

	let mut entries: Vec<_> = map.iter().collect();
	entries.sort_by(|left, right| left.0.cmp(right.0));

	for (index, (key, value)) in entries.into_iter().enumerate() {
		if index > 0 {
			out.push(b',');
		}

		out.extend(serde_json::to_vec(key).expect("a JSON key serializes"));
		out.push(b':');
		canonical_json(value, out);
	}

	out.push(b'}');
}

fn canonical_array(values: &[serde_json::Value], out: &mut Vec<u8>) {
	out.push(b'[');

	for (index, value) in values.iter().enumerate() {
		if index > 0 {
			out.push(b',');
		}

		canonical_json(value, out);
	}

	out.push(b']');
}

/// The canonical SHA-256 of a non-secret plugin option table, already encoded
/// as JSON.
pub fn options_digest(options: &serde_json::Value) -> String {
	let mut out = Vec::new();
	canonical_json(options, &mut out);
	hash_bytes(&out)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn descriptor() -> HostCapabilityDescriptor {
		HostCapabilityDescriptor {
			format_version: CAPABILITY_CONTRACT_VERSION,
			platform: "ios".into(),
			flavor: "ios".into(),
			experimental: false,
			protocol: ProtocolVersions {
				guest_abi: 1,
				bridge: 1,
			},
			capabilities: vec![
				CapabilityEntry {
					id: CAP_HOST_PLUGIN_RUST_SYNC.into(),
					state: CapabilityState::Available,
					detail: None,
				},
				CapabilityEntry {
					id: CAP_HOST_PLUGIN_RUST_ASYNC.into(),
					state: CapabilityState::Available,
					detail: None,
				},
			],
			plugins: vec![PluginCapability {
				name: "http".into(),
				state: CapabilityState::Experimental,
				implementation: Some("rust.async".into()),
				option_keys: vec!["origins".into()],
				options_sha256: Some(hash_bytes(b"options")),
				detail: None,
			}],
		}
	}

	#[test]
	fn digest_ignores_entry_and_map_order() {
		let left = descriptor();
		let mut right = descriptor();
		right.capabilities.reverse();
		right.plugins.reverse();
		assert_eq!(left.digest(), right.digest());

		// Canonical JSON sorts object keys even when serde_json was built with
		// an insertion-order map.
		let mut unordered = serde_json::Map::new();
		unordered.insert("z".into(), serde_json::json!(1));
		unordered.insert("a".into(), serde_json::json!(2));

		let mut ordered = serde_json::Map::new();
		ordered.insert("a".into(), serde_json::json!(2));
		ordered.insert("z".into(), serde_json::json!(1));

		let mut unordered_bytes = Vec::new();
		let mut ordered_bytes = Vec::new();
		canonical_json(&serde_json::Value::Object(unordered), &mut unordered_bytes);
		canonical_json(&serde_json::Value::Object(ordered), &mut ordered_bytes);
		assert_eq!(unordered_bytes, ordered_bytes);

		let mut empty = descriptor();
		empty.capabilities.clear();
		empty.plugins.clear();
		assert_ne!(descriptor().digest(), empty.digest());
	}

	#[test]
	fn rejects_duplicate_and_conflicting_entries() {
		let mut duplicate = descriptor();
		duplicate
			.capabilities
			.push(duplicate.capabilities[0].clone());
		assert!(matches!(
			duplicate.validate(),
			Err(CodePushError::InvalidCapabilityContract(message))
				if message.contains("more than once")
		));

		let mut duplicate_plugins = descriptor();
		duplicate_plugins
			.plugins
			.push(duplicate_plugins.plugins[0].clone());
		assert!(duplicate_plugins.validate().is_err());

		let mut unsorted = descriptor();
		unsorted.plugins[0].option_keys = vec!["z".into(), "a".into()];
		assert!(unsorted.validate().is_err());

		let mut conflicting = descriptor();
		conflicting.capabilities.push(CapabilityEntry {
			id: "plugin.http".into(),
			state: CapabilityState::Available,
			detail: None,
		});
		assert!(matches!(
			conflicting.validate(),
			Err(CodePushError::InvalidCapabilityContract(message))
				if message.contains("conflicts")
		));

		let mut requirements = RuntimeRequirements {
			format_version: CAPABILITY_CONTRACT_VERSION,
			protocol: ProtocolVersions {
				guest_abi: 1,
				bridge: 1,
			},
			required: vec!["plugin.http".into()],
			optional: vec!["plugin.http".into()],
		};
		assert!(requirements.validate().is_err());
		requirements.optional.clear();
		requirements.required.push("plugin.http".into());
		assert!(requirements.validate().is_err());
	}

	#[test]
	fn rejects_unknown_versions_and_bad_identifiers() {
		let mut version = descriptor();
		version.format_version = CAPABILITY_CONTRACT_VERSION + 1;
		assert!(matches!(
			version.validate(),
			Err(CodePushError::UnsupportedContractVersion(_))
		));

		let mut identifier = descriptor();
		identifier.platform = "IOS".into();
		assert!(identifier.validate().is_err());

		let requirements = RuntimeRequirements {
			format_version: RUNTIME_CONTRACT_VERSION,
			protocol: ProtocolVersions {
				guest_abi: 1,
				bridge: 1,
			},
			required: Vec::new(),
			optional: Vec::new(),
		};
		assert!(matches!(
			requirements.validate(),
			Err(CodePushError::UnsupportedContractVersion(_))
		));
	}

	#[test]
	fn required_capabilities_gate_and_optional_ones_do_not() {
		let host = descriptor();
		let required = RuntimeRequirements {
			format_version: CAPABILITY_CONTRACT_VERSION,
			protocol: ProtocolVersions {
				guest_abi: 1,
				bridge: 1,
			},
			required: vec![CAP_HOST_PLUGIN_RUST_ASYNC.into(), "plugin.http".into()],
			optional: vec!["plugin.missing".into()],
		};
		assert!(required.verify_against(&host).is_ok());

		let unsupported = RuntimeRequirements {
			format_version: CAPABILITY_CONTRACT_VERSION,
			protocol: ProtocolVersions {
				guest_abi: 1,
				bridge: 1,
			},
			required: vec!["host.plugin.native.apple".into()],
			optional: Vec::new(),
		};
		assert!(unsupported.verify_against(&host).is_err());

		let unknown = RuntimeRequirements {
			format_version: CAPABILITY_CONTRACT_VERSION,
			protocol: ProtocolVersions {
				guest_abi: 1,
				bridge: 1,
			},
			required: vec!["plugin.not-built".into()],
			optional: Vec::new(),
		};
		assert!(unknown.verify_against(&host).is_err());
	}

	#[test]
	fn options_digest_is_canonical() {
		let left = serde_json::json!({"z": 1, "a": "x"});
		let right = serde_json::json!({"a": "x", "z": 1});
		assert_eq!(options_digest(&left), options_digest(&right));

		let changed = serde_json::json!({"a": "x", "z": 2});
		assert_ne!(options_digest(&left), options_digest(&changed));
	}
}
