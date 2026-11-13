use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::graph::Unit;

use super::capture::Selected;

pub(super) mod inputs;
pub(super) struct Built {
	pub(super) binary: PathBuf,
	pub(super) inputs: inputs::Inputs,
}

pub(super) struct Builder<'a> {
	root: &'a Path,
	metadata: Option<crate::wasm::CargoMetadata>,
	services: BTreeMap<String, crate::services::Service>,
	release: bool,
	inputs: BTreeMap<String, inputs::Inputs>,
}

impl<'a> Builder<'a> {
	pub(super) fn prepare(
		root: &'a Path,
		config: &LeptoskitConfig,
		selected: &Selected,
		release: bool,
	) -> Result<Self> {
		let names: Vec<_> = selected
			.selection
			.stages
			.iter()
			.flatten()
			.filter_map(|unit| {
				match unit {
					Unit::Service(name) => Some(name),
					Unit::Frontend(_) => None,
				}
			})
			.collect();
		let mut result = Self {
			root,
			metadata: None,
			services: BTreeMap::new(),
			inputs: BTreeMap::new(),
			release,
		};

		if names.is_empty() {
			return Ok(result);
		}

		let metadata = crate::wasm::cargo_metadata(root)?;

		for name in names {
			let mut one = config.clone();
			one.services.retain(|key, _| key == name);
			// The resolver's legacy collision check has no application listener
			// in this lifecycle. Port zero is never a valid declared service
			// port.
			one.serve.port = 0;
			let mut services = crate::services::resolve(&one, root, &metadata, release)?;
			let service = services
				.pop()
				.context("selected service was not resolved")?;
			ensure!(
				!service.settings.database
					&& service.settings.pool.is_none()
					&& service.settings.volumes.is_empty(),
				"graph service {name} requires undeclared persistent data"
			);
			ensure!(
				!service.settings.public
					&& service.settings.routes.is_empty()
					&& service.settings.bind.is_empty(),
				"graph service {name} inherits unsupported shared-app public/routes/bind settings"
			);

			let inputs = inputs::Inputs::prepare(root, &metadata, &service, release)?;
			result.inputs.insert(name.clone(), inputs);
			result.services.insert(name.clone(), service);
		}

		result.metadata = Some(metadata);
		Ok(result)
	}

	pub(super) fn build(&mut self, name: &str) -> Result<Built> {
		let service = self
			.services
			.get(name)
			.context("service was not prepared")?;
		let inputs = self
			.inputs
			.remove(name)
			.context("service inputs already consumed")?;
		inputs.recheck()?;
		// Independent service artifacts must not inherit another service's
		// Cargo features through a multi-package invocation.
		let built = crate::services::build(
			self.root,
			self.metadata.as_ref().context("missing Cargo metadata")?,
			None,
			std::slice::from_ref(service),
			self.release,
		)?;
		let binary = built
			.services
			.into_iter()
			.next()
			.and_then(|service| service.binary)
			.context("service build produced no executable")?;
		inputs.recheck()?;
		Ok(Built { binary, inputs })
	}
}
