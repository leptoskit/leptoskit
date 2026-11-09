//! Normalize explicit declarations without reading artifacts or contacting
//! providers. Filesystem, pinned-image and runtime checks belong to the
//! subsequent concrete build/release plan.

use std::collections::BTreeMap;
use std::path::Component;
use std::path::Path;

use super::Engine;
use super::FrontendKind;
use super::InfraGraph;
use crate::LeptoskitConfig;
use crate::graph::Unit;
use crate::graph::artifacts::Tree;
use crate::graph::plan::Call;
use crate::graph::plan::ContainerRuntime;
use crate::graph::plan::Host;
use crate::graph::plan::Placement;
use crate::graph::plan::Plan;
use crate::graph::routes::Route;

/// Compile the explicitly selected graph lifecycle. `None` means an
/// existing single-app project; graph-only declarations in that lifecycle
/// fail rather than being ignored.
///
/// # Errors
/// Returns the declaration or ownership conflict before any build, provider
/// call, file write, runtime access or infrastructure operation.
pub fn compile(config: &LeptoskitConfig) -> Result<Option<Plan>, String> {
	let graph = config
		.infra
		.as_ref()
		.and_then(crate::infra::InfraConfig::as_graph);
	let Some(graph) = graph else {
		if graph_fields(config) {
			return Err(
				"artifacts, frontends and independently deployed services require infra.mode = \
				 'graph'"
					.into(),
			);
		}

		return Ok(None);
	};

	if let Some(problem) = graph.problem() {
		return Err(problem);
	}

	if config.db.is_some()
		|| config.auth.is_some()
		|| config.build.backend != Some(false)
		|| config.serve.probes
	{
		return Err(
			"graph projects require build.backend = false and no root db/auth/probes; declare \
			 application processes and their data on their own units"
				.into(),
		);
	}
	let mut builder = Builder::new(graph, config)?;

	for (name, frontend) in &config.frontends {
		builder.frontend(name, frontend)?;
	}

	for (name, service) in &config.services {
		builder.service(name, service)?;
	}

	builder.finish().map(Some)
}

struct Builder<'a> {
	graph: &'a InfraGraph,
	artifacts: Vec<String>,
	hosts: BTreeMap<String, Host>,
	placements: Vec<Placement>,
	routes: Vec<Route>,
	trees: Vec<Tree>,
	calls: Vec<Call>,
	ports: BTreeMap<(String, u16), Unit>,
}

impl<'a> Builder<'a> {
	fn new(graph: &'a InfraGraph, config: &LeptoskitConfig) -> Result<Self, String> {
		artifact_outputs(config)?;

		Ok(Self {
			graph,
			artifacts: config.artifacts.keys().cloned().collect(),
			hosts: graph
				.hosts
				.iter()
				.map(|(name, host)| {
					(
						name.clone(),
						Host {
							name: name.clone(),
							exclusive: host.exclusive,
							container_runtime: None,
						},
					)
				})
				.collect(),
			placements: Vec::new(),
			routes: Vec::new(),
			trees: Vec::new(),
			calls: Vec::new(),
			ports: BTreeMap::new(),
		})
	}

	fn frontend(&mut self, name: &str, frontend: &super::Frontend) -> Result<(), String> {
		let unit = Unit::Frontend(name.to_owned());

		match frontend.kind {
			FrontendKind::Static if frontend.backend.is_some() || frontend.serve.is_some() => {
				return Err(format!(
					"{unit} is static and cannot declare a backend or server probes"
				));
			}

			FrontendKind::Ssr => {
				if frontend.backend.as_ref().is_none_or(|path| !relative(path)) {
					return Err(format!(
						"{unit}.backend must name an executable inside its artifact output"
					));
				}

				let serve = frontend
					.serve
					.as_ref()
					.filter(|serve| serve.probes)
					.ok_or_else(|| {
						format!("{unit} needs an explicit serve.probes readiness contract")
					})?;
				listen_variable(&unit, serve.address_env.as_deref())?;
				self.port(&unit, &frontend.host, serve.port)?;
			}
			FrontendKind::Static => {}
		}

		self.route(&unit, &frontend.host, &frontend.domain, &frontend.mount)?;
		self.trees.push(Tree {
			unit: unit.clone(),
			artifact: frontend.artifact.clone(),
			directory: frontend.directory.clone(),
		});
		let placement = placement(
			unit,
			&frontend.host,
			vec![frontend.artifact.clone()],
			&frontend.depends_on,
		)?;
		self.call_edges(&placement.unit, &frontend.calls)?;
		self.placements.push(placement);
		Ok(())
	}

	fn service(&mut self, name: &str, service: &crate::ServiceConfig) -> Result<(), String> {
		let unit = Unit::Service(name.to_owned());
		service_options(name, service, &unit)?;
		let deploy = service
			.deploy
			.as_ref()
			.ok_or_else(|| format!("{unit}.deploy must declare its independent host"))?;
		let port = service
			.port
			.ok_or_else(|| format!("{unit}.port must declare its listener"))?;
		self.port(&unit, &deploy.host, port)?;

		if !service.serve.as_ref().is_some_and(|serve| serve.probes) {
			return Err(format!(
				"{unit} needs an explicit serve.probes readiness contract"
			));
		}

		listen_variable(
			&unit,
			service
				.serve
				.as_ref()
				.and_then(|serve| serve.address_env.as_deref()),
		)?;
		if let Some(domain) = &deploy.domain {
			self.route(&unit, &deploy.host, domain, &deploy.mount)?;
		} else if deploy.mount != "/" || deploy.ingress.is_some() {
			return Err(format!("{unit}.deploy mount/ingress needs a public domain"));
		}

		if let Some(ingress) = deploy.ingress
			&& self
				.graph
				.edge
				.as_ref()
				.is_none_or(|edge| edge.ingress != ingress)
		{
			return Err(format!(
				"{unit}.deploy.ingress conflicts with the project's edge ingress"
			));
		}
		let artifact = self.implicit_artifact("service", name)?;
		let mut placement = placement(unit, &deploy.host, vec![artifact], &deploy.depends_on)?;

		if let Some(runtime) = &service.runtime {
			self.runtime(name, service, &runtime.container, &mut placement)?;
		}

		self.call_edges(&placement.unit, &deploy.calls)?;
		self.placements.push(placement);
		Ok(())
	}

	fn runtime(
		&mut self,
		name: &str,
		service: &crate::ServiceConfig,
		container: &super::Container,
		placement: &mut Placement,
	) -> Result<(), String> {
		if !container.rootless || !relative(&container.image_pin) {
			return Err(format!(
				"services.{name}.runtime.container requires rootless=true and a project-relative \
				 image-pin file"
			));
		}

		runtime_bindings(name, service, container)?;

		let engine = match container.engine {
			Engine::Docker => ContainerRuntime::RootlessDocker,
			Engine::Podman => ContainerRuntime::RootlessPodman,
		};

		let host = self
			.hosts
			.get_mut(&placement.host)
			.ok_or_else(|| format!("services.{name}.deploy.host is undeclared"))?;

		if host
			.container_runtime
			.is_some_and(|current| current != engine)
		{
			return Err(format!(
				"host {} has conflicting container runtime requirements",
				host.name
			));
		}
		host.container_runtime = Some(engine);
		placement.requires_exclusive_host = true;
		placement.container_runtime = Some(engine);
		placement
			.artifacts
			.push(self.implicit_artifact("toolchain", name)?);
		Ok(())
	}

	fn implicit_artifact(&mut self, kind: &str, service: &str) -> Result<String, String> {
		let name = format!("{kind}-{service}");

		if self.artifacts.contains(&name) {
			return Err(format!(
				"artifact {name} is reserved by services.{service}; rename the explicit artifact"
			));
		}

		self.artifacts.push(name.clone());
		Ok(name)
	}

	fn route(&mut self, unit: &Unit, host: &str, domain: &str, mount: &str) -> Result<(), String> {
		if self.graph.edge.is_none() {
			return Err(format!(
				"{unit} has a public route but infra.edge is absent"
			));
		}

		self.routes.push(Route {
			unit: unit.clone(),
			host: host.into(),
			domain: domain.into(),
			mount: mount.into(),
		});
		Ok(())
	}

	fn port(&mut self, unit: &Unit, host: &str, port: u16) -> Result<(), String> {
		if port == 0 {
			return Err(format!("{unit} must listen on a nonzero port"));
		}

		if let Some(previous) = self.ports.insert((host.to_owned(), port), unit.clone()) {
			return Err(format!(
				"{previous} and {unit} both use port {port} on host {host}"
			));
		}

		Ok(())
	}

	fn call_edges(&mut self, unit: &Unit, targets: &[String]) -> Result<(), String> {
		for target in targets {
			self.calls.push(Call {
				caller: unit.clone(),
				target: unit_reference(target)?,
			});
		}

		Ok(())
	}

	fn finish(self) -> Result<Plan, String> {
		Plan::new(
			self.hosts.into_values().collect(),
			self.artifacts,
			self.placements,
			self.routes,
			self.trees,
			self.calls,
		)
	}
}

fn listen_variable(unit: &Unit, value: Option<&str>) -> Result<(), String> {
	if value.is_some_and(|name| !crate::infra::env_name(name)) {
		return Err(format!(
			"{unit}.serve.address-env must be an uppercase environment variable"
		));
	}
	Ok(())
}

fn placement(
	unit: Unit,
	host: &str,
	artifacts: Vec<String>,
	dependencies: &[String],
) -> Result<Placement, String> {
	Ok(Placement {
		unit,
		host: host.to_owned(),
		artifacts,
		depends_on: dependencies
			.iter()
			.map(|name| unit_reference(name))
			.collect::<Result<_, _>>()?,
		requires_exclusive_host: false,
		container_runtime: None,
	})
}

fn unit_reference(value: &str) -> Result<Unit, String> {
	match value.split_once('.') {
		Some(("frontends", name)) if crate::graph::identifier(name) => {
			Ok(Unit::Frontend(name.into()))
		}

		Some(("services", name)) if crate::graph::identifier(name) => {
			Ok(Unit::Service(name.into()))
		}

		_ => {
			Err(format!(
				"unit reference {value:?} must be frontends.<name> or services.<name>"
			))
		}
	}
}

fn relative(path: &Path) -> bool {
	path.is_relative()
		&& path.to_str().is_some_and(|value| {
			!value.is_empty()
				&& value.split('/').all(|part| {
					!part.is_empty()
						&& part != "."
						&& part != ".."
						&& !part
							.bytes()
							.any(|byte| byte.is_ascii_control() || byte == b'\\')
				})
		})
		&& path
			.components()
			.all(|part| matches!(part, Component::Normal(_)))
}

// These legacy fields describe a colocated root application. Reject them
// explicitly until the graph has an equivalent ownership and recovery contract.
fn service_options(name: &str, service: &crate::ServiceConfig, unit: &Unit) -> Result<(), String> {
	if let Some(problem) = service.problem(name) {
		return Err(problem);
	}

	if service.url.is_some() {
		return Err(format!(
			"{unit}: external service bindings need an explicit external graph node; URL-only \
			 services cannot be deployed as local units"
		));
	}

	if service.public.is_some() || service.routes.is_some() {
		return Err(format!(
			"{unit}: use deploy.domain and deploy.mount instead of the shared-app public/routes \
			 preset"
		));
	}

	if service.database != Some(false)
		|| service.pool.is_some()
		|| service
			.volumes
			.as_ref()
			.is_some_and(|volumes| !volumes.is_empty())
	{
		return Err(format!(
			"{unit}: declare database=false; persistent graph data needs explicit ownership and \
			 recovery support before deployment"
		));
	}

	if !service.bind.is_empty() {
		return Err(format!(
			"{unit}: app-level bind variables have no root application in graph mode"
		));
	}

	Ok(())
}

fn runtime_bindings(
	name: &str,
	service: &crate::ServiceConfig,
	container: &super::Container,
) -> Result<(), String> {
	let variables = [
		&container.socket_env,
		&container.engine_env,
		&container.image_env,
	];

	for (index, variable) in variables.iter().enumerate() {
		if !crate::infra::env_name(variable)
			|| variables[..index].contains(variable)
			|| service.env.contains_key(*variable)
			|| service
				.secrets
				.as_ref()
				.is_some_and(|secrets| secrets.contains(variable))
		{
			return Err(format!(
				"services.{name}.runtime binding {variable:?} must be a distinct unshadowed \
				 environment name"
			));
		}
	}

	Ok(())
}

fn artifact_outputs(config: &LeptoskitConfig) -> Result<(), String> {
	let mut outputs: Vec<(&str, &Path)> = Vec::new();

	for (name, artifact) in &config.artifacts {
		if artifact.command.first().is_none_or(String::is_empty)
			|| artifact.command.iter().any(|part| part.contains('\0'))
		{
			return Err(format!(
				"artifacts.{name}.command needs a program and NUL-free arguments"
			));
		}

		if !relative(&artifact.output) {
			return Err(format!(
				"artifacts.{name}.output must be a non-root relative directory inside the project"
			));
		}

		for (other, output) in &outputs {
			if artifact.output.starts_with(output) || output.starts_with(&artifact.output) {
				return Err(format!(
					"artifacts.{name}.output overlaps artifacts.{other}; share one producer and \
					 partition its output between frontends"
				));
			}
		}

		outputs.push((name, &artifact.output));
	}

	Ok(())
}

fn graph_fields(config: &LeptoskitConfig) -> bool {
	!config.artifacts.is_empty()
		|| !config.frontends.is_empty()
		|| config.services.values().any(|service| {
			service.deploy.is_some() || service.serve.is_some() || service.runtime.is_some()
		})
}
