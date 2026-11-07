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

use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;
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
	/// every required capability.
	Verified,
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
		if self.format_version != CAPABILITY_CONTRACT_VERSION {
			return Err(CodePushError::UnsupportedContractVersion(
				self.format_version,
			));
		}

		if !capability_identifier(&self.platform) || !capability_identifier(&self.flavor) {
			return Err(invalid("platform and flavor must be stable identifiers"));
		}

		if self.protocol.guest_abi == 0 || self.protocol.bridge == 0 {
			return Err(invalid("protocol versions must be positive"));
		}

		let mut capabilities = std::collections::BTreeSet::new();

		for capability in &self.capabilities {
			if !capability_identifier(&capability.id) {
				return Err(invalid(format!(
					"`{}` is not a stable capability identifier",
					capability.id
				)));
			}

			if !capabilities.insert(capability.id.as_str()) {
				return Err(invalid(format!(
					"capability `{}` is declared more than once",
					capability.id
				)));
			}
		}

		let mut plugins = std::collections::BTreeSet::new();

		for plugin in &self.plugins {
			if !capability_identifier(&plugin.name) {
				return Err(invalid(format!(
					"plugin `{}` is not a stable identifier",
					plugin.name
				)));
			}

			if !plugins.insert(plugin.name.as_str()) {
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

			if let Some(digest) = &plugin.options_sha256
				&& !is_sha256(digest)
			{
				return Err(invalid(format!(
					"plugin `{}` options digest must be lowercase hex SHA-256",
					plugin.name
				)));
			}
		}

		for capability in &self.capabilities {
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
			let mut seen = std::collections::BTreeSet::new();

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

fn is_sha256(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Canonical JSON with sorted object keys, so map iteration order can never
/// change a digest.
fn canonical_json(value: &serde_json::Value, out: &mut Vec<u8>) {
	match value {
		serde_json::Value::Object(map) => {
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
		serde_json::Value::Array(values) => {
			out.push(b'[');

			for (index, value) in values.iter().enumerate() {
				if index > 0 {
					out.push(b',');
				}

				canonical_json(value, out);
			}

			out.push(b']');
		}
		other => out.extend(serde_json::to_vec(other).expect("a JSON value serializes")),
	}
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
