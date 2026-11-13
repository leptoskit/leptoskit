//! Release contracts derived from checked artifacts and explicit declarations.
//! Neither the declaration nor ELF inspection proves a running service.
use std::collections::BTreeMap;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment::FrontendKind;
use leptoskit_config::graph::Unit;
use serde_json::Value;
use serde_json::json;

use crate::verify::assets::Member;
use crate::verify::graph::Checked;
use crate::verify::graph::Executable;
use crate::verify::graph::linux::Architecture;

pub(super) struct Bound {
	pub executables: BTreeMap<String, Value>,
	pub formats: BTreeMap<String, Value>,
}

pub(super) fn bind(
	config: &LeptoskitConfig,
	checked: &Checked,
	units: &BTreeMap<String, Value>,
	architecture: Architecture,
) -> Result<Bound> {
	let selected = units.keys().cloned().collect::<Vec<_>>();
	let mut result = Bound {
		executables: BTreeMap::new(),
		formats: BTreeMap::new(),
	};
	for executable in checked.executable_inventory(&selected)? {
		ensure!(
			units.contains_key(&executable.unit),
			"executable is outside selected host"
		);
		let contract = contract(config, &executable)?;
		let format = checked.executable_format(&executable.unit, architecture)?;
		result
			.formats
			.insert(executable.unit.clone(), serde_json::to_value(format)?);
		result.executables.insert(executable.unit, contract);
	}
	Ok(result)
}

fn listener<'a>(
	config: &'a LeptoskitConfig,
	executable: &Executable,
) -> Result<(u16, Option<&'a str>, &'a str)> {
	let unit = super::super::unit(&executable.unit)?;
	Ok(match &unit {
		Unit::Frontend(name) => {
			let frontend = config
				.graph_infra()
				.context("graph infrastructure required")?
				.frontends
				.get(name)
				.context("frontend declaration missing")?;
			ensure!(
				frontend.kind == FrontendKind::Ssr && executable.service.is_none(),
				"backend is not an SSR frontend"
			);
			let serve = frontend
				.serve
				.as_ref()
				.context("SSR serve contract missing")?;
			ensure!(
				serve.probes && serve.address == "127.0.0.1",
				"graph SSR requires loopback address and readiness probes"
			);
			(
				serve.port,
				serve.address_env.as_deref(),
				frontend.mount.as_str(),
			)
		}
		Unit::Service(name) => {
			let declared = config
				.services
				.get(name)
				.context("service declaration missing")?;
			let settings = &executable
				.service
				.as_ref()
				.context("checked service source evidence missing")?
				.settings;
			ensure!(settings.name == *name, "captured service identity differs");
			ensure!(
				!settings.public && settings.routes.is_empty(),
				"inherited public/routes use the alongside-app routing contract; remove them from \
				 package metadata and declare deploy.domain/deploy.mount"
			);
			ensure!(
				!settings.database
					&& settings.pool.is_none()
					&& settings.volumes.is_empty()
					&& settings.bind.is_empty(),
				"service data and private bindings require an ownership lifecycle"
			);
			ensure!(
				settings.secrets.is_empty(),
				"service secret delivery is unsupported"
			);
			ensure!(
				declared.runtime.is_none(),
				"rootless runtime delivery is unsupported"
			);
			let serve = declared
				.serve
				.as_ref()
				.filter(|s| s.probes)
				.context("service readiness contract missing")?;
			let deploy = declared
				.deploy
				.as_ref()
				.context("service placement missing")?;
			let port = settings.port.context("resolved service port missing")?;
			ensure!(
				declared.port == Some(port),
				"resolved service port differs from graph routing"
			);
			(port, serve.address_env.as_deref(), deploy.mount.as_str())
		}
	})
}

fn contract(config: &LeptoskitConfig, executable: &Executable) -> Result<Value> {
	let unit = super::super::unit(&executable.unit)?;
	let (port, address_env, mount) = listener(config, executable)?;
	let variable = address_env
		.context("executable graph release requires serve.address-env consumed by the backend")?;
	ensure!(
		leptoskit_config::infra::env_name(variable),
		"invalid serve.address-env"
	);
	ensure!(
		port >= 1024 && !matches!(port, 8088 | 18088),
		"application port is reserved or privileged"
	);
	ensure!(
		mount == "/",
		"mounted executable probes need an explicit path contract; static path mounts remain \
		 supported"
	);
	let Member::File {
		mode: 0o755,
		size,
		sha256,
	} = &executable.backend
	else {
		anyhow::bail!("checked backend must be executable mode 0755");
	};
	ensure!(
		*size > 0 && *size <= 64 * 1024 * 1024,
		"backend exceeds release transport limit"
	);
	let project = super::super::lifecycle::settings(config)?;
	let address = format!("127.0.0.1:{port}");
	let mut environment = executable
		.service
		.as_ref()
		.map(|service| service.settings.env.clone())
		.unwrap_or_default();
	ensure!(
		!environment.contains_key(variable),
		"service env must not override serve.address-env"
	);
	// The worker allocates a fresh port. Reserve the longest possible address
	// so allocation cannot take a validated environment over its byte budget.
	environment.insert(variable.to_owned(), "127.0.0.1:65535".into());
	crate::verify::graph::service::environment::validate(&environment)?;
	environment.insert(variable.to_owned(), address.clone());
	Ok(
		json!({"account":super::super::account(&project.name, &unit),
		"address":address,"address_env":variable,"environment":environment,
		"backend":{"mode":0o755,"size":size,"sha256":sha256},
		"health":"/healthz","ready":"/readyz","runtime":null}),
	)
}

#[cfg(test)]
mod tests;
