//! Deliver an approved window to a managed server without changing its bytes.

use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::infra::InfraConfig;
use leptoskit_db::MigrationWindow;
use serde_json::Value;
use serde_json::json;

use super::remote::Reservation;
use super::remote::{self};

const CONTAINER_PATH: &str = "/run/leptoskit/migration-window.json";
const PATH_ENV: &str = "LEPTOSKIT_MIGRATION_WINDOW";
const PIN_ENV: &str = "LEPTOSKIT_MIGRATION_WINDOW_SHA256";

/// Constructed only from validated, independently pinned window bytes.
pub(super) struct Window {
	json: String,
	pin: String,
}

impl Window {
	pub(super) fn new(json: &str, pin: &str) -> Result<Self> {
		MigrationWindow::from_json_pinned(json, pin)?;
		Ok(Self {
			json: json.into(),
			pin: pin.into(),
		})
	}

	fn host_path(&self, name: &str) -> Result<String> {
		Ok(format!("{}/{}.json", directory(name)?, self.pin))
	}

	fn install(&self, hosts: &[String], name: &str, reservation: &Reservation) -> Result<()> {
		let script = self.install_script(name)?;

		for host in hosts {
			reservation.check()?;
			let observed = remote::execute(host, &script, Some(self.json.as_bytes()))?;
			ensure!(
				String::from_utf8(observed)?.trim() == self.pin,
				"host did not retain the approved migration window"
			);
		}

		reservation.check()
	}

	/// Never overwrite an existing digest-named file, even if it is corrupt.
	/// The source file is readable by the unprivileged container; its parent
	/// remains private to the operator. Managed hosts use Linux coreutils.
	fn install_script(&self, name: &str) -> Result<String> {
		Ok(self.install_script_at(&directory(name)?, &self.host_path(name)?))
	}

	fn install_script_at(&self, directory: &str, file: &str) -> String {
		let directory = remote::quote(directory);
		let file = remote::quote(file);
		let pin = remote::quote(&self.pin);
		format!(
			r#"set -eu
umask 077
directory={directory}
file={file}
pin={pin}
mkdir -p "$directory"
test -d "$directory" && test ! -L "$directory"
chmod 700 "$directory"
temporary=$(mktemp "$directory/.incoming.XXXXXXXX")
trap 'rm -f "$temporary"' EXIT
cat > "$temporary"
test "$(sha256sum "$temporary" | cut -d ' ' -f 1)" = "$pin"
chmod 444 "$temporary"
sync -f "$temporary"
if ! ln -T -- "$temporary" "$file" 2>/dev/null; then
  test -f "$file" && test ! -L "$file"
fi
test -f "$file" && test ! -L "$file"
test "$(stat -c %a "$file")" = 444
test "$(sha256sum "$file" | cut -d ' ' -f 1)" = "$pin"
sync -f "$directory"
printf '%s' "$pin"
"#
		)
	}

	pub(super) fn verify_running(&self, hosts: &[String], name: &str, version: &str) -> Result<()> {
		super::build::validate_version(version)?;
		let file = self.host_path(name)?;
		let container = remote::quote(&format!("{name}-web-{version}"));

		for host in hosts {
			let bytes = remote::execute(host, &format!("docker inspect {container}"), None)?;
			let containers: Vec<Value> = serde_json::from_slice(&bytes)?;
			ensure!(
				containers.len() == 1,
				"missing or ambiguous running window container"
			);
			self.check_container(name, &containers[0])?;
			let observed = remote::execute(
				host,
				&format!(
					"set -eu; test -f {file} && test ! -L {file}; test \"$(stat -c %a {file})\" = \
					 444; sha256sum {file} | cut -d ' ' -f 1",
					file = remote::quote(&file),
				),
				None,
			)?;
			ensure!(
				String::from_utf8(observed)?.trim() == self.pin,
				"running server's migration window changed"
			);
		}

		Ok(())
	}

	fn check_container(&self, name: &str, container: &Value) -> Result<()> {
		ensure!(
			container["State"]["Running"] == true,
			"window server is not running"
		);
		let environment = container["Config"]["Env"]
			.as_array()
			.context("missing running environment")?;

		for (key, expected) in [(PATH_ENV, CONTAINER_PATH), (PIN_ENV, self.pin.as_str())] {
			let values: Vec<_> = environment
				.iter()
				.filter_map(Value::as_str)
				.filter_map(|entry| entry.split_once('='))
				.filter_map(|(name, value)| (name == key).then_some(value))
				.collect();
			ensure!(
				values == [expected],
				"running server does not use the approved migration window configuration"
			);
		}

		let mounts = container["Mounts"]
			.as_array()
			.context("missing running mounts")?;
		let matching: Vec<_> = mounts
			.iter()
			.filter(|mount| mount["Destination"] == CONTAINER_PATH)
			.collect();
		ensure!(
			matching.len() == 1,
			"migration window needs exactly one bind mount"
		);
		let mount = matching[0];
		ensure!(
			mount["Type"] == "bind"
				&& mount["RW"] == false
				&& mount["Source"] == self.host_path(name)?,
			"migration window mount is writable or has the wrong source"
		);
		Ok(())
	}
}

fn directory(name: &str) -> Result<String> {
	ensure!(
		!name.is_empty()
			&& !name.starts_with('-')
			&& name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
		"invalid managed application name"
	);
	Ok(format!("{}/schema-windows", remote::base(name)))
}

/// Also used when recovering a previous release: its window replaces the
/// failed candidate's configuration, or the reserved fields are removed.
pub(super) fn prepare(
	dir: &Path,
	config: &InfraConfig,
	hosts: &[String],
	window: Option<&Window>,
	reservation: &Reservation,
) -> Result<()> {
	let path = dir.join("release.yml");
	let source = fs::read_to_string(&path)?;
	let configured = configure(&source, &config.name, window)?;

	if window.is_none() && configured == source {
		return Ok(());
	}

	if let Some(window) = window {
		window.install(hosts, &config.name, reservation)?;
	}

	reservation.check()?;
	fs::write(path, configured)?;
	Ok(())
}

/// Rebuild only the reserved window fields, refusing unrecognized mounts
/// so recovery cannot silently replace an operator-managed binding.
pub(super) fn configure(source: &str, name: &str, window: Option<&Window>) -> Result<String> {
	let prefix = format!("{}/", directory(name)?);
	let suffix = format!(":{CONTAINER_PATH}:ro");
	let mut config: Value = serde_json::from_str(source)?;
	let secrets = config["env"]["secret"]
		.as_array()
		.context("invalid runtime secret names")?;
	ensure!(
		!secrets
			.iter()
			.any(|name| name == PATH_ENV || name == PIN_ENV),
		"migration window variables are reserved for approved release evidence"
	);
	let clear = config["env"]["clear"]
		.as_object_mut()
		.context("invalid runtime environment")?;
	clear.remove(PATH_ENV);
	clear.remove(PIN_ENV);

	if let Some(window) = window {
		clear.insert(PATH_ENV.into(), json!(CONTAINER_PATH));
		clear.insert(PIN_ENV.into(), json!(window.pin));
	}

	let mut retained = Vec::new();

	if let Some(volumes) = config.get("volumes") {
		for volume in volumes.as_array().context("invalid runtime volumes")? {
			let text = volume.as_str().context("invalid runtime volume")?;

			if text.split(':').nth(1) == Some(CONTAINER_PATH) {
				let pin = text
					.strip_prefix(&prefix)
					.and_then(|text| text.strip_suffix(&suffix))
					.and_then(|file| file.strip_suffix(".json"));
				ensure!(
					pin.is_some_and(super::verified::valid_digest),
					"unrecognized migration window mount; refusing to replace it"
				);
			} else {
				retained.push(volume.clone());
			}
		}
	}

	if let Some(window) = window {
		retained.push(json!(format!(
			"{}:{CONTAINER_PATH}:ro",
			window.host_path(name)?
		)));
	}

	if retained.is_empty() {
		config
			.as_object_mut()
			.context("invalid runtime configuration")?
			.remove("volumes");
	} else {
		config["volumes"] = json!(retained);
	}

	Ok(serde_json::to_string_pretty(&config)? + "\n")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod acceptance;
