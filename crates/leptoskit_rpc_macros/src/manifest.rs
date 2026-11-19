//! What the manifest of the crate an attribute expands in says about its
//! features.
//!
//! A service's operations and channels keep their server items behind the
//! declaring crate's own `service` feature. A crate that only calls a
//! service, such as an app, has no such feature, and Cargo's check-cfg warns
//! about `cfg(feature = "service")` there. Without the feature that `cfg` is
//! always false, so the attributes give such a crate the stub directly.
//!
//! An app crate can also keep its own operations' server items behind one of
//! its features, so that a crate which links it for the client only, while
//! `leptoskit_rpc/server` is on for some other reason, neither compiles nor
//! serves them. It names the feature in its manifest:
//!
//! ```toml
//! [package.metadata.leptoskit.rpc]
//! server-feature = "server"
//! ```

use std::path::Path;

/// Whether the manifest of the crate being compiled declares a `service`
/// feature. Cargo sets `CARGO_MANIFEST_DIR` to that crate's directory, and
/// recompiles the crate when its declared features change. Without a
/// readable manifest the answer is yes, which keeps the `cfg` as it was.
pub(crate) fn declares_service_feature() -> bool {
	declares_service_in(
		std::env::var_os("CARGO_MANIFEST_DIR")
			.as_deref()
			.map(Path::new),
	)
}

/// [`declares_service_feature`] for the crate in `directory`.
fn declares_service_in(directory: Option<&Path>) -> bool {
	directory.is_none_or(|directory| {
		std::fs::read_to_string(directory.join("Cargo.toml"))
			.map_or(true, |manifest| declares_service(&manifest))
	})
}

/// Whether `manifest`, a `Cargo.toml`, has `service` in its `[features]`. A
/// manifest that doesn't parse counts as declaring it: Cargo has already
/// rejected it, and the `cfg` stays as it was.
fn declares_service(manifest: &str) -> bool {
	manifest.parse::<toml::Table>().map_or(true, |manifest| {
		manifest
			.get("features")
			.and_then(toml::Value::as_table)
			.is_some_and(|features| features.contains_key("service"))
	})
}

/// The feature the crate being compiled keeps its own operations' server
/// items behind, from `[package.metadata.leptoskit.rpc] server-feature`, or
/// `None` when it names none. Cargo doesn't rebuild a crate when only its
/// metadata changes, so `cargo clean -p <crate>` after adding or removing it.
///
/// # Errors
///
/// When the key isn't a string, or names a feature the manifest doesn't
/// declare, so a typo can't silently leave the bodies in every build.
pub(crate) fn server_feature() -> Result<Option<String>, String> {
	let Some(directory) = std::env::var_os("CARGO_MANIFEST_DIR") else {
		return Ok(None);
	};

	match std::fs::read_to_string(Path::new(&directory).join("Cargo.toml")) {
		Ok(manifest) => server_feature_in(&manifest),
		Err(_) => Ok(None),
	}
}

/// [`server_feature`] for `manifest`, a `Cargo.toml`. A manifest that doesn't
/// parse names none: Cargo has already rejected it.
fn server_feature_in(manifest: &str) -> Result<Option<String>, String> {
	let Ok(manifest) = manifest.parse::<toml::Table>() else {
		return Ok(None);
	};
	let Some(value) = manifest
		.get("package")
		.and_then(|package| package.get("metadata"))
		.and_then(|metadata| metadata.get("leptoskit"))
		.and_then(|leptoskit| leptoskit.get("rpc"))
		.and_then(|rpc| rpc.get("server-feature"))
	else {
		return Ok(None);
	};
	let Some(feature) = value.as_str() else {
		return Err(
			"`[package.metadata.leptoskit.rpc] server-feature` must name one of this crate's \
			 features, as a string"
				.to_owned(),
		);
	};
	let declared = manifest
		.get("features")
		.and_then(toml::Value::as_table)
		.is_some_and(|features| features.contains_key(feature));

	if !declared {
		return Err(format!(
			"`[package.metadata.leptoskit.rpc] server-feature` names `{feature}`, which isn't in \
			 this crate's `[features]`"
		));
	}

	Ok(Some(feature.to_owned()))
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use super::declares_service;
	use super::declares_service_feature;
	use super::declares_service_in;
	use super::server_feature;
	use super::server_feature_in;

	#[test]
	fn a_service_feature_is_found_among_the_features() {
		assert!(declares_service(
			"[package]\nname = \"analytics\"\n\n[features]\ndefault = []\nservice = \
			 [\"leptoskit_rpc/postgres\"]\n"
		));
	}

	#[test]
	fn a_crate_without_the_feature_does_not_declare_it() {
		assert!(!declares_service("[package]\nname = \"app\"\n"));
		assert!(!declares_service(
			"[package]\nname = \"app\"\n\n[features]\nssr = [\"leptoskit_rpc/services\"]\n"
		));
		// A dependency or another table named `service` is not the feature.
		assert!(!declares_service(
			"[package]\nname = \"app\"\n\n[dependencies]\nservice = \"1\"\n"
		));
		assert!(!declares_service("features = \"service\"\n"));
	}

	#[test]
	fn a_manifest_that_does_not_parse_keeps_the_cfg() {
		assert!(declares_service("[features\nservice = []"));
	}

	#[test]
	fn the_macro_crate_itself_has_no_service_feature() {
		// Tests run with `CARGO_MANIFEST_DIR` set to this crate, which
		// declares `ssr` and `server` but no `service`.
		assert!(!declares_service_feature());
	}

	#[test]
	fn without_a_readable_manifest_the_cfg_is_kept() {
		assert!(declares_service_in(None));
		assert!(declares_service_in(Some(Path::new(
			"/nonexistent/leptoskit_rpc_macros"
		))));
	}

	#[test]
	fn a_server_feature_is_read_from_the_metadata() {
		assert_eq!(
			server_feature_in(
				"[package]\nname = \"cloud\"\n\n[package.metadata.leptoskit.rpc]\nserver-feature \
				 = \"server\"\n\n[features]\nserver = []\n"
			),
			Ok(Some("server".to_owned()))
		);
	}

	#[test]
	fn without_the_metadata_there_is_no_server_feature() {
		assert_eq!(
			server_feature_in("[package]\nname = \"app\"\n\n[features]\nserver = []\n"),
			Ok(None)
		);
		assert_eq!(server_feature_in("[package\nname = \"app\"\n"), Ok(None));
		// The macro crate itself names none.
		assert_eq!(server_feature(), Ok(None));
	}

	#[test]
	fn a_server_feature_must_be_a_declared_feature() {
		let undeclared = server_feature_in(
			"[package]\nname = \"cloud\"\n\n[package.metadata.leptoskit.rpc]\nserver-feature = \
			 \"sever\"\n\n[features]\nserver = []\n",
		)
		.expect_err("an undeclared feature");
		assert!(undeclared.contains("`sever`"), "{undeclared}");

		let not_a_string = server_feature_in(
			"[package]\nname = \"cloud\"\n\n[package.metadata.leptoskit.rpc]\nserver-feature = \
			 true\n",
		)
		.expect_err("not a string");
		assert!(not_a_string.contains("as a string"), "{not_a_string}");
	}
}
