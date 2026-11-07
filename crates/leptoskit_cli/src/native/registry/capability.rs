//! The versioned host capability descriptor built from the registry that the
//! destination actually resolved.
//!
//! The descriptor is the one model the build writes, the update check
//! compares, and the platform capability documentation records. It is derived
//! from [`HostCapabilities`], each selected plugin's [`Implementation`], the
//! app's plugin options, and the explicit required/optional declarations. It
//! never reads a downloaded guest's labels and never contains machine paths,
//! timestamps, or option values.

use leptoskit_codepush::capability::CAP_HOST_PLATFORM_EXPERIMENTAL;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_NATIVE_ANDROID;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_NATIVE_APPLE;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_NATIVE_ARKTS;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_RUST_ASYNC;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_RUST_SYNC;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_RUST_WORKER;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_SERVICE;
use leptoskit_codepush::capability::CAP_HOST_UPDATE_CODEPUSH;
use leptoskit_codepush::capability::CAPABILITY_CONTRACT_VERSION;
use leptoskit_codepush::capability::CapabilityEntry;
use leptoskit_codepush::capability::CapabilityState;
use leptoskit_codepush::capability::HostCapabilityDescriptor;
use leptoskit_codepush::capability::PluginCapability;
use leptoskit_codepush::capability::ProtocolVersions;
use leptoskit_codepush::capability::RuntimeRequirements;
use leptoskit_codepush::capability::options_digest;
use leptoskit_codepush::capability::plugin_capability;

use super::*;

/// Build the descriptor for `target` from the plugins the registry kept and
/// the ones it left out.
pub(crate) fn descriptor(
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
	plugins: &[Plugin],
	missing: &[MissingPlugin],
) -> HostCapabilityDescriptor {
	let host = HostCapabilities::of(target);
	let mut capabilities = host_capabilities(target, host);
	let mut plugin_entries = Vec::new();

	for name in &config.native.plugins {
		let entry = plugin_entry(config, target, host, plugins, missing, name);
		plugin_entries.push(entry);
	}

	// A declaration the host does not describe is explicit, never silently
	// treated as available.
	for id in config
		.native
		.required_capabilities
		.iter()
		.chain(&config.native.optional_capabilities)
	{
		if !capabilities.iter().any(|entry| &entry.id == id)
			&& !plugin_entries.iter().any(|entry| {
				id.strip_prefix("plugin.")
					.is_some_and(|name| entry.name == name)
			}) {
			capabilities.push(CapabilityEntry {
				id: id.clone(),
				state: CapabilityState::Unknown,
				detail: Some(
					"this build does not describe the capability; check its spelling or upgrade \
					 leptoskit"
						.into(),
				),
			});
		}
	}

	capabilities.sort_by(|left, right| left.id.cmp(&right.id));
	plugin_entries.sort_by(|left, right| left.name.cmp(&right.name));

	HostCapabilityDescriptor {
		format_version: CAPABILITY_CONTRACT_VERSION,
		platform: target.platform().to_string(),
		flavor: target.flavor().into(),
		experimental: target.experimental(),
		protocol: ProtocolVersions {
			guest_abi: leptoskit_wasm::ABI_VERSION,
			bridge: leptoskit_bridge::VERSION,
		},
		capabilities,
		plugins: plugin_entries,
	}
}

/// The signed guest requirements: explicit required plugins/capabilities and
/// optional ones. Unlisted grants keep their existing `missing-plugins`
/// behavior and are not new requirements.
pub(crate) fn requirements(config: &leptoskit_config::LeptoskitConfig) -> RuntimeRequirements {
	let required = config
		.native
		.required_plugins
		.iter()
		.map(|name| plugin_capability(name))
		.chain(config.native.required_capabilities.iter().cloned())
		.collect::<std::collections::BTreeSet<_>>()
		.into_iter()
		.collect();
	let optional = config
		.native
		.optional_plugins
		.iter()
		.map(|name| plugin_capability(name))
		.chain(config.native.optional_capabilities.iter().cloned())
		.collect::<std::collections::BTreeSet<_>>()
		.into_iter()
		.collect();

	RuntimeRequirements {
		format_version: CAPABILITY_CONTRACT_VERSION,
		required,
		optional,
	}
}

/// Fail the build before packaging when an explicit required capability or
/// plugin cannot run on this destination.
pub(crate) fn enforce_required(
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
	plugins: &[Plugin],
	missing: &[MissingPlugin],
) -> Result<()> {
	let descriptor = descriptor(config, target, plugins, missing);

	for id in &requirements(config).required {
		let Some((state, detail)) = descriptor.state_of(id) else {
			anyhow::bail!(
				"native.required-capabilities declares `{id}`, but the {} {} does not describe \
				 the capability; check its spelling or upgrade leptoskit",
				platform_label(target),
				host_noun(target)
			);
		};

		if !state.usable() {
			let detail = detail.map_or_else(String::new, |detail| format!(": {detail}"));
			anyhow::bail!(
				"native.required-capabilities declares `{id}`, but the {} {} reports it \
				 {state:?}{detail}; make it available, move it to the optional list, or drop the \
				 platform",
				platform_label(target),
				host_noun(target)
			);
		}
	}

	Ok(())
}

/// Actionable warnings for optional capabilities and plugins this destination
/// leaves out. They never fail the build.
pub(crate) fn optional_warnings(
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
	plugins: &[Plugin],
	missing: &[MissingPlugin],
) -> Vec<String> {
	let descriptor = descriptor(config, target, plugins, missing);
	let required = requirements(config).required;
	let mut warnings = Vec::new();
	let mut optional_plugins = std::collections::BTreeSet::new();

	for name in &config.native.optional_plugins {
		optional_plugins.insert(plugin_capability(name));
	}

	for id in requirements(config).optional {
		if required.contains(&id) {
			continue;
		}

		let Some((state, detail)) = descriptor.state_of(&id) else {
			warnings.push(format!(
				"warning: optional capability `{id}` is not described by this leptoskit build; \
				 check its spelling or upgrade leptoskit"
			));
			continue;
		};

		if state.usable() {
			continue;
		}

		let detail = detail.map_or_else(String::new, |detail| format!(" ({detail})"));

		if optional_plugins.contains(&id) {
			let name = id.strip_prefix("plugin.").unwrap_or(&id);
			warnings.push(format!(
				"warning: optional runtime plugin `{name}` is not available on {}{detail}; calls \
				 fail with Unavailable",
				platform_label(target)
			));
		} else {
			warnings.push(format!(
				"warning: optional capability `{id}` is not available on {}{detail}; the app must \
				 degrade without it",
				platform_label(target)
			));
		}
	}

	warnings.sort();
	warnings.dedup();
	warnings
}

/// The destination matrix rendered from the same [`HostCapabilities`] model
/// that the descriptor uses. `docs/agents/platform-capability-matrix.md` is
/// checked against this in a unit test, so documentation cannot drift from the
/// host support the registry actually applies.
#[cfg(test)]
pub(crate) fn documentation_table() -> String {
	const TARGETS: [Target; 12] = [
		Target::Macos,
		Target::IosSimulator,
		Target::Ios,
		Target::Android,
		Target::AndroidX86_64,
		Target::Harmonyos,
		Target::HarmonyosX86_64,
		Target::Wearos,
		Target::Watchos,
		Target::Windows,
		Target::Linux,
		Target::Tui,
	];
	let yes = |value: bool| if value { "yes" } else { "no" };
	let mut out = String::from(
		"# Platform capability matrix\n\nGenerated from the same host support model as \
		 `capabilities.json`; do not edit by hand. Declared build support is not runtime \
		 evidence.\n\n| Flavour | Platform | Experimental | Sync Rust | Worker Rust | Async Rust \
		 | Service | Native | Code push |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- \
		 |\n",
	);

	for target in TARGETS {
		let host = HostCapabilities::of(target);
		let native = match host.native {
			Some(NativeKind::Apple) => "apple",
			Some(NativeKind::Android) => "android",
			Some(NativeKind::ArkTs) => "arkts",
			None => "—",
		};
		out.push_str(&format!(
			"| `{}` | `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
			target.flavor(),
			target.platform(),
			yes(target.experimental()),
			yes(host.sync),
			yes(host.worker),
			yes(host.asynchronous),
			yes(host.service),
			native,
			yes(runs_code_push(target))
		));
	}

	out
}

fn runs_code_push(target: Target) -> bool {
	matches!(
		target,
		Target::Ios | Target::IosSimulator | Target::Android | Target::AndroidX86_64
	)
}

fn host_capabilities(target: Target, host: HostCapabilities) -> Vec<CapabilityEntry> {
	let mut entries = vec![
		entry(
			CAP_HOST_PLUGIN_RUST_SYNC,
			host.sync,
			&format!(
				"the {} {} does not run synchronous Rust plugins",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_PLUGIN_RUST_WORKER,
			host.worker,
			&format!(
				"the {} {} does not run worker or main-thread plugins",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_PLUGIN_RUST_ASYNC,
			host.asynchronous,
			&format!(
				"the {} {} does not run async plugins",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_PLUGIN_SERVICE,
			host.service,
			&format!(
				"the {} {} does not run plugin services",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_PLUGIN_NATIVE_APPLE,
			host.native == Some(NativeKind::Apple),
			&format!(
				"the {} {} does not dispatch AppKit or UIKit plugin types",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_PLUGIN_NATIVE_ANDROID,
			host.native == Some(NativeKind::Android),
			&format!(
				"the {} {} does not dispatch Kotlin plugin types",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_PLUGIN_NATIVE_ARKTS,
			host.native == Some(NativeKind::ArkTs),
			&format!(
				"the {} {} does not dispatch ArkTS plugin types",
				platform_label(target),
				host_noun(target)
			),
		),
		entry(
			CAP_HOST_UPDATE_CODEPUSH,
			runs_code_push(target),
			"automatic code push checks run only in the generated iOS and Android hosts",
		),
	];

	if target.experimental() {
		entries.push(CapabilityEntry {
			id: CAP_HOST_PLATFORM_EXPERIMENTAL.into(),
			state: CapabilityState::Experimental,
			detail: Some(format!(
				"{} is an experimental destination",
				platform_label(target)
			)),
		});
	}

	entries
}

fn entry(id: &str, available: bool, detail: &str) -> CapabilityEntry {
	CapabilityEntry {
		id: id.into(),
		state: if available {
			CapabilityState::Available
		} else {
			CapabilityState::Unsupported
		},
		detail: (!available).then(|| detail.into()),
	}
}

fn plugin_entry(
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
	host: HostCapabilities,
	plugins: &[Plugin],
	missing: &[MissingPlugin],
	name: &str,
) -> PluginCapability {
	let options = config.native.plugin_options.get(name);
	let option_keys = options
		.map(|options| {
			let mut keys: Vec<_> = options.keys().cloned().collect();
			keys.sort();
			keys
		})
		.unwrap_or_default();
	let options_sha256 = options.map(|options| {
		options_digest(&serde_json::to_value(options).expect("TOML options serialize as JSON"))
	});

	let (state, implementation, detail) =
		if let Some(plugin) = plugins.iter().find(|plugin| &plugin.name == name) {
			match plugin.implementation(target, host) {
				Ok(implementation) => {
					(
						if plugin.uses_experimental_rust(target) {
							CapabilityState::Experimental
						} else {
							CapabilityState::Available
						},
						Some(implementation_id(&implementation)),
						None,
					)
				}
				Err(reason) => unsupported(reason),
			}
		} else if let Some(missing) = missing.iter().find(|missing| &missing.name == name) {
			unsupported(missing.reason)
		} else if matches!(target, Target::Windows | Target::Linux | Target::Tui) {
			(
				CapabilityState::Unsupported,
				None,
				Some(format!(
					"the {} runner does not run runtime plugins yet",
					platform_label(target)
				)),
			)
		} else {
			(
				CapabilityState::Unknown,
				None,
				Some(format!(
					"this build did not resolve the plugin for {}",
					platform_label(target)
				)),
			)
		};

	PluginCapability {
		name: name.into(),
		state,
		implementation,
		option_keys,
		options_sha256,
		detail,
	}
}

fn unsupported(reason: Missing) -> (CapabilityState, Option<String>, Option<String>) {
	let detail = match reason {
		Missing::NoImplementation => "has no implementation for this destination",
		Missing::ExperimentalRustPlugin => {
			"requires native.experimental-rust-plugins = true; the only implementation is \
			 experimental"
		}
		Missing::HostCannotRun(capability) => {
			match capability {
				Capability::Sync => "the host does not run synchronous Rust plugins yet",
				Capability::Worker => "the host does not run worker or main-thread plugins yet",
				Capability::Async => "the host does not run async plugins yet",
				Capability::Service => "the host does not run plugin services yet",
			}
		}
		Missing::Unavailable => "the plugin opts this destination out",
	};

	(CapabilityState::Unsupported, None, Some(detail.into()))
}

fn implementation_id(implementation: &Implementation<'_>) -> String {
	match implementation {
		Implementation::Rust(rust) => {
			match rust.flavour {
				Flavour::Sync => "rust.sync".into(),
				Flavour::Worker => "rust.worker".into(),
				Flavour::MainThread => "rust.main-thread".into(),
				Flavour::Async => "rust.async".into(),
			}
		}
		Implementation::Apple { .. } => "native.apple".into(),
		Implementation::Android { .. } => "native.android".into(),
		Implementation::ArkTs { .. } => "native.arkts".into(),
		Implementation::Service(_) => "service".into(),
		Implementation::Hybrid { native, .. } => {
			format!("service+{}", implementation_id(native))
		}
	}
}

fn platform_label(target: Target) -> &'static str {
	match target {
		Target::Ios | Target::IosSimulator => "iOS",
		Target::Macos => "macOS",
		Target::Android | Target::AndroidX86_64 => "Android",
		Target::Harmonyos | Target::HarmonyosX86_64 => "HarmonyOS",
		Target::Wearos => "Wear OS",
		Target::Watchos => "watchOS",
		Target::Windows => "Windows",
		Target::Linux => "Linux",
		Target::Tui => "the terminal",
	}
}

fn host_noun(target: Target) -> &'static str {
	match target {
		Target::Windows | Target::Linux | Target::Tui => "runner",
		_ => "host",
	}
}
