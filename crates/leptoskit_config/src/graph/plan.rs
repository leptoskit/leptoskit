//! A common ownership and ordering plan for frontend and service releases.
//!
//! Provider configuration and observed runtime capabilities are separate
//! inputs to deployment. This module validates declared requirements; it
//! never upgrades a declaration into evidence that a host meets them.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::Unit;
use super::artifacts::Tree;
use super::artifacts::Trees;
use super::routes::Route;
use super::routes::Routes;

/// A runtime the operator asks the platform to install for trusted services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContainerRuntime {
	RootlessDocker,
	RootlessPodman,
}

/// Host ownership. Provider size/region and observed capabilities belong
/// to the provider plan and acceptance evidence, respectively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
	pub name: String,
	/// One unit owns the entire host. A frontend is also a unit and cannot
	/// be silently placed beside an exclusive service.
	pub exclusive: bool,
	pub container_runtime: Option<ContainerRuntime>,
}

/// Placement and readiness dependencies for one independently released unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
	pub unit: Unit,
	pub host: String,
	pub requires_exclusive_host: bool,
	pub container_runtime: Option<ContainerRuntime>,
	/// Named build inputs, including a backend or pinned toolchain artifact.
	/// A shared producer is built once even when several units consume it.
	pub artifacts: Vec<String>,
	/// Only actual startup/readiness prerequisites. Merely calling another
	/// service does not require its release to precede this one.
	pub depends_on: Vec<Unit>,
}

/// A call relationship, separate from readiness ordering. Cycles here are
/// valid. Authentication, bindings and compatibility are checked when the
/// complete release plan is assembled.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Call {
	pub caller: Unit,
	pub target: Unit,
}

/// Validated common graph. It contains only declarations and ordering, not
/// verified artifacts, authorized releases, migrations or running state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
	hosts: BTreeMap<String, Host>,
	artifacts: BTreeSet<String>,
	placements: BTreeMap<Unit, Placement>,
	routes: Routes,
	trees: Trees,
	calls: Vec<Call>,
	stages: Vec<Vec<Unit>>,
}

impl Plan {
	/// Join placement, public routes, build outputs and call relationships.
	///
	/// # Errors
	/// Rejects dangling references, duplicate or conflicting ownership,
	/// unsupported declared runtimes, exclusive-host violations and readiness
	/// cycles before any build or infrastructure operation.
	pub fn new(
		hosts: Vec<Host>,
		artifacts: Vec<String>,
		placements: Vec<Placement>,
		routes: Vec<Route>,
		trees: Vec<Tree>,
		calls: Vec<Call>,
	) -> Result<Self, String> {
		let hosts = hosts_by_name(hosts)?;
		let placements = placements_by_unit(&hosts, placements)?;
		let artifacts = artifact_names(artifacts)?;
		let routes = Routes::new(routes)?;
		let trees = Trees::new(trees)?;
		validate_artifact_inputs(&artifacts, &placements)?;
		validate_routes(&routes, &placements)?;
		validate_trees(&trees, &placements)?;
		let calls = call_edges(calls, &placements)?;
		let stages = stages(&placements)?;
		Ok(Self {
			hosts,
			artifacts,
			placements,
			routes,
			trees,
			calls,
			stages,
		})
	}

	#[must_use]
	pub fn hosts(&self) -> &BTreeMap<String, Host> {
		&self.hosts
	}

	/// All named artifact inputs, independently of the current selection.
	#[must_use]
	pub fn artifacts(&self) -> &BTreeSet<String> {
		&self.artifacts
	}

	#[must_use]
	pub fn placements(&self) -> &BTreeMap<Unit, Placement> {
		&self.placements
	}

	#[must_use]
	pub fn routes(&self) -> &Routes {
		&self.routes
	}

	#[must_use]
	pub fn trees(&self) -> &Trees {
		&self.trees
	}

	#[must_use]
	pub fn calls(&self) -> &[Call] {
		&self.calls
	}

	/// Units in one stage have no unsatisfied readiness dependencies between
	/// them. A scheduler may still run them serially to respect resource
	/// limits.
	#[must_use]
	pub fn stages(&self) -> &[Vec<Unit>] {
		&self.stages
	}

	/// Select exactly the requested release units, ordered by their existing
	/// readiness relationships. Dependencies outside the selection are returned
	/// separately for fresh readiness validation; they are never
	/// auto-restarted.
	///
	/// # Errors
	/// An empty selection or an unknown/repeated unit is an error.
	pub fn select(&self, units: &[Unit]) -> Result<Selection, String> {
		let selected: BTreeSet<_> = units.iter().cloned().collect();

		if selected.is_empty() || selected.len() != units.len() {
			return Err("release selection must name distinct units".into());
		}

		for unit in &selected {
			if !self.placements.contains_key(unit) {
				return Err(format!("release selection names undeclared {unit}"));
			}
		}

		let mut readiness = BTreeSet::new();
		let mut artifacts = BTreeSet::new();

		let mut pending: Vec<_> = selected.iter().cloned().collect();
		let mut visited = BTreeSet::new();

		while let Some(unit) = pending.pop() {
			if !visited.insert(unit.clone()) {
				continue;
			}

			for dependency in &self.placements[&unit].depends_on {
				if !selected.contains(dependency) {
					readiness.insert(dependency.clone());
				}

				pending.push(dependency.clone());
			}
		}

		for unit in &selected {
			artifacts.extend(self.placements[unit].artifacts.iter().cloned());
		}

		let stages = self
			.stages
			.iter()
			.map(|stage| {
				stage
					.iter()
					.filter(|unit| selected.contains(*unit))
					.cloned()
					.collect::<Vec<_>>()
			})
			.filter(|stage| !stage.is_empty())
			.collect();
		Ok(Selection {
			stages,
			readiness: readiness.into_iter().collect(),
			artifacts: artifacts.into_iter().collect(),
		})
	}
}

/// A release selection. Shared build recipes are deduplicated without
/// adding unrelated units to the deployment selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
	pub stages: Vec<Vec<Unit>>,
	pub readiness: Vec<Unit>,
	pub artifacts: Vec<String>,
}

fn artifact_names(artifacts: Vec<String>) -> Result<BTreeSet<String>, String> {
	let mut result = BTreeSet::new();

	for artifact in artifacts {
		if !super::artifact_identifier(&artifact) {
			return Err(format!("invalid graph artifact name {artifact:?}"));
		}

		if !result.insert(artifact.clone()) {
			return Err(format!("artifact {artifact} is declared twice"));
		}
	}

	Ok(result)
}

fn validate_artifact_inputs(
	artifacts: &BTreeSet<String>,
	placements: &BTreeMap<Unit, Placement>,
) -> Result<(), String> {
	for placement in placements.values() {
		for artifact in &placement.artifacts {
			if !artifacts.contains(artifact) {
				return Err(format!(
					"{} names undeclared artifact {artifact}",
					placement.unit
				));
			}
		}
	}

	Ok(())
}

fn validate_routes(routes: &Routes, placements: &BTreeMap<Unit, Placement>) -> Result<(), String> {
	for route in routes.entries() {
		let placement = placements
			.get(&route.unit)
			.ok_or_else(|| format!("public route names undeclared {}", route.unit))?;

		if placement.host != route.host {
			return Err(format!(
				"{} is placed on {}, but its route uses {}",
				route.unit, placement.host, route.host
			));
		}
	}

	Ok(())
}

fn validate_trees(trees: &Trees, placements: &BTreeMap<Unit, Placement>) -> Result<(), String> {
	for tree in trees.entries() {
		let placement = placements
			.get(&tree.unit)
			.ok_or_else(|| format!("artifact tree names undeclared {}", tree.unit))?;

		if !placement.artifacts.contains(&tree.artifact) {
			return Err(format!(
				"{} does not declare its artifact tree's input {}",
				tree.unit, tree.artifact
			));
		}
	}

	Ok(())
}

fn call_edges(
	mut calls: Vec<Call>,
	placements: &BTreeMap<Unit, Placement>,
) -> Result<Vec<Call>, String> {
	calls.sort();

	for (index, call) in calls.iter().enumerate() {
		if !placements.contains_key(&call.caller) || !placements.contains_key(&call.target) {
			return Err(format!(
				"call {} -> {} names an undeclared unit",
				call.caller, call.target
			));
		}

		if index > 0 && calls[index - 1] == *call {
			return Err(format!(
				"call {} -> {} is declared twice",
				call.caller, call.target
			));
		}
	}

	Ok(calls)
}

fn hosts_by_name(hosts: Vec<Host>) -> Result<BTreeMap<String, Host>, String> {
	let mut result = BTreeMap::new();

	for host in hosts {
		if !super::identifier(&host.name) {
			return Err(format!("invalid graph host name {:?}", host.name));
		}

		if let Some(previous) = result.insert(host.name.clone(), host) {
			return Err(format!("host {} is declared twice", previous.name));
		}
	}

	Ok(result)
}

fn placements_by_unit(
	hosts: &BTreeMap<String, Host>,
	placements: Vec<Placement>,
) -> Result<BTreeMap<Unit, Placement>, String> {
	let mut result = BTreeMap::new();
	let mut counts = BTreeMap::<&str, usize>::new();

	for mut placement in placements {
		placement.unit.validate()?;
		let host = hosts.get(&placement.host).ok_or_else(|| {
			format!(
				"{} names undeclared host {}",
				placement.unit, placement.host
			)
		})?;
		validate_placement(&mut placement, host)?;
		let count = counts.entry(&host.name).or_default();
		*count += 1;

		if host.exclusive && *count > 1 {
			return Err(format!(
				"exclusive host {} has several release units",
				host.name
			));
		}

		if let Some(previous) = result.insert(placement.unit.clone(), placement) {
			return Err(format!("{} is declared twice", previous.unit));
		}
	}

	if result.is_empty() {
		return Err("deployment graph has no release units".into());
	}

	Ok(result)
}

fn validate_placement(placement: &mut Placement, host: &Host) -> Result<(), String> {
	if placement.requires_exclusive_host && !host.exclusive {
		return Err(format!("{} requires an exclusive host", placement.unit));
	}

	if placement.container_runtime.is_some()
		&& placement.container_runtime != host.container_runtime
	{
		return Err(format!(
			"{} requires a container runtime not declared by host {}",
			placement.unit, host.name
		));
	}
	placement.depends_on.sort();
	placement.artifacts.sort();

	if placement
		.artifacts
		.windows(2)
		.any(|pair| pair[0] == pair[1])
	{
		return Err(format!("{} repeats an artifact input", placement.unit));
	}

	if placement
		.depends_on
		.windows(2)
		.any(|pair| pair[0] == pair[1])
	{
		return Err(format!("{} repeats a readiness dependency", placement.unit));
	}
	Ok(())
}

// Readiness is a DAG: remove one complete ready stage at a time. Call edges
// never enter this loop, and an empty stage with pending units is a cycle.
fn stages(placements: &BTreeMap<Unit, Placement>) -> Result<Vec<Vec<Unit>>, String> {
	for placement in placements.values() {
		for dependency in &placement.depends_on {
			if !placements.contains_key(dependency) {
				return Err(format!(
					"{} depends on undeclared {dependency}",
					placement.unit
				));
			}
		}
	}

	let mut pending: BTreeSet<_> = placements.keys().cloned().collect();
	let mut result = Vec::new();

	while !pending.is_empty() {
		let ready: Vec<_> = pending
			.iter()
			.filter(|unit| {
				placements[*unit]
					.depends_on
					.iter()
					.all(|dependency| !pending.contains(dependency))
			})
			.cloned()
			.collect();

		if ready.is_empty() {
			return Err(format!(
				"readiness dependencies contain a cycle among {}",
				pending
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>()
					.join(", ")
			));
		}

		for unit in &ready {
			pending.remove(unit);
		}

		result.push(ready);
	}

	Ok(result)
}

#[cfg(test)]
mod tests;
