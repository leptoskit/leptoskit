//! Whether the crate an attribute expands in declares a `service` feature.
//!
//! A service's operations and channels keep their server items behind the
//! declaring crate's own `service` feature. A crate that only calls a
//! service, such as an app, has no such feature, and Cargo's check-cfg warns
//! about `cfg(feature = "service")` there. Without the feature that `cfg` is
//! always false, so the attributes give such a crate the stub directly.

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

#[cfg(test)]
mod tests {
	use std::path::Path;

	use super::declares_service;
	use super::declares_service_feature;
	use super::declares_service_in;

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
}
