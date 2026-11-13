use std::collections::BTreeMap;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment::FrontendKind;
use leptoskit_config::graph::Unit;
use leptoskit_config::graph::plan::Plan;
use serde::Deserialize;

use crate::verify::assets;
use crate::verify::assets::Member;
use crate::verify::assets::Root;
use crate::verify::assets::Tree;

mod unique;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
	format_version: u32,
	kind: String,
	pub(super) configuration_sha256: String,
	pub(super) service_profile: String,
	cli_sha256: String,
	recipes: BTreeMap<String, String>,
	pub(super) units: BTreeMap<String, Captured>,
	readiness_only: Vec<String>,
	evidence: CaptureEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureEvidence {
	artifact_integrity: String,
	source_reproduction: String,
	isolated_build: String,
	origin_authorization: String,
	runtime: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Captured {
	pub(super) service: Option<super::service::ServiceEvidence>,
	host: String,
	pub(super) assets: Option<Tree>,
	pub(super) backend: Option<Member>,
	toolchain_image: Option<String>,
}

pub(super) fn parse(bytes: &[u8]) -> Result<Manifest> {
	let problem = |error: serde_json::Error| {
		anyhow::anyhow!(
			"invalid graph receipt at line {}, column {}",
			error.line(),
			error.column()
		)
	};
	serde_json::from_slice::<unique::Unique>(bytes).map_err(problem)?;
	let result: Manifest = serde_json::from_slice(bytes).map_err(problem)?;
	let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(problem)?;
	for (name, captured) in &result.units {
		if let Some(service) = &captured.service {
			ensure!(
				raw["units"][name]["service"]["settings"]
					== serde_json::to_value(&service.settings)?,
				"noncanonical or unknown service settings fields"
			);
		}
	}
	Ok(result)
}

impl Manifest {
	pub(super) fn validate(
		&self,
		project: &Root,
		config: &LeptoskitConfig,
		plan: &Plan,
	) -> Result<()> {
		ensure!(
			matches!(self.format_version, 1 | 2) && self.kind == "leptoskit-graph-build",
			"unsupported graph receipt format"
		);
		ensure!(
			self.configuration_sha256 == super::hash(&serde_json::to_vec(config)?),
			"graph receipt configuration differs from this project"
		);
		ensure!(
			matches!(self.service_profile.as_str(), "dev" | "release")
				&& crate::verify::digest_is_valid(&self.cli_sha256),
			"invalid graph build profile or CLI digest"
		);
		let evidence = &self.evidence;
		ensure!(
			evidence.artifact_integrity == "captured"
				&& [
					&evidence.source_reproduction,
					&evidence.isolated_build,
					&evidence.origin_authorization,
					&evidence.runtime
				]
				.into_iter()
				.all(|value| value == "not_checked"),
			"ordinary graph build receipts cannot claim stronger verification evidence"
		);
		let units = self
			.units
			.keys()
			.map(|name| unit(name))
			.collect::<Result<Vec<_>>>()?;
		let selection = plan.select(&units).map_err(anyhow::Error::msg)?;
		ensure!(
			self.readiness_only
				== selection
					.readiness
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>(),
			"graph receipt readiness dependencies differ"
		);
		let recipes = selection
			.artifacts
			.iter()
			.filter_map(|name| config.artifacts.get(name).map(|recipe| (name, recipe)))
			.map(|(name, recipe)| Ok((name.clone(), super::hash(&serde_json::to_vec(recipe)?))))
			.collect::<Result<BTreeMap<_, _>>>()?;
		ensure!(self.recipes == recipes, "graph receipt recipes differ");
		let mut entries = 0_usize;
		let mut bytes = 0_u64;
		for (name, captured) in &self.units {
			let unit = unit(name)?;
			ensure!(
				self.format_version == 2 || captured.service.is_none(),
				"format one cannot contain service evidence"
			);
			ensure!(
				!matches!(unit, Unit::Service(_))
					|| (self.format_version == 2 && captured.service.is_some()),
				"legacy service receipt lacks executable source evidence; rebuild required"
			);
			ensure!(
				captured.host == plan.placements()[&unit].host,
				"graph receipt host placement differs"
			);
			captured.declaration(project, config, plan, &unit)?;
			if let Some(service) = &captured.service {
				assets::validate(&service.inputs)?;
				ensure!(service.inputs.len() <= 256, "too many service inputs");
				let mut total = 0u64;
				for (path, member) in &service.inputs {
					ensure!(!path.contains('/'), "service inputs must be flat files");
					let Member::File { size, .. } = member else {
						anyhow::bail!("service inputs must be files")
					};
					total = total
						.checked_add(*size)
						.context("service input size overflow")?;
					ensure!(
						total <= assets::MAX_METADATA_BYTES,
						"service input byte budget exceeded"
					);
				}
			}
			for member in captured
				.assets
				.iter()
				.flat_map(|tree| tree.values())
				.chain(captured.backend.iter())
				.chain(
					captured
						.service
						.iter()
						.flat_map(|service| service.inputs.values()),
				) {
				entries = entries
					.checked_add(1)
					.filter(|count| *count <= assets::MAX_ENTRIES)
					.context("graph receipt has too many members")?;
				if let Member::File { size, .. } = member {
					bytes = bytes
						.checked_add(*size)
						.filter(|total| *total <= assets::MAX_BYTES)
						.context("graph receipt exceeds its aggregate byte limit")?;
				}
			}
		}
		Ok(())
	}
}

impl Captured {
	fn declaration(
		&self,
		project: &Root,
		config: &LeptoskitConfig,
		plan: &Plan,
		unit: &Unit,
	) -> Result<()> {
		if let Some(tree) = &self.assets {
			assets::validate(tree)?;
		}
		if let Some(backend) = &self.backend {
			ensure!(
				matches!(backend, Member::File {mode:0o755,size,sha256}
				if *size > 0 && *size <= assets::MAX_BYTES && crate::verify::digest_is_valid(sha256)),
				"graph backend requires nonempty executable bytes and a SHA-256"
			);
		}
		match unit {
			Unit::Frontend(name) => {
				let frontend = &config
					.graph_infra()
					.context("graph infrastructure required")?
					.frontends[name];
				let assets = self
					.assets
					.as_ref()
					.context("frontend has no asset inventory")?;
				let mut excluded = plan.trees().exclusions(unit).map_err(anyhow::Error::msg)?;
				if let Some(backend) = &frontend.backend {
					let backend = backend.to_str().context("backend path is not UTF-8")?;
					if frontend.directory == "." {
						excluded.push(backend.into());
					} else if let Some(relative) = backend
						.strip_prefix(&frontend.directory)
						.and_then(|path| path.strip_prefix('/'))
					{
						excluded.push(relative.into());
					}
				}
				ensure!(
					assets.keys().all(|path| {
						excluded.iter().all(|excluded| {
							path != excluded
								&& !path
									.strip_prefix(excluded)
									.is_some_and(|rest| rest.starts_with('/'))
						})
					}),
					"frontend receipt includes another unit's tree or its private executable"
				);
				ensure!(
					self.toolchain_image.is_none() && self.service.is_none(),
					"frontend cannot claim a service toolchain"
				);
				ensure!(
					self.backend.is_some() == (frontend.kind == FrontendKind::Ssr),
					"frontend executable differs from its declared kind"
				);
				if frontend.kind == FrontendKind::Static {
					ensure!(
						matches!(assets.get("index.html"),Some(Member::File {size,..}) if *size>0),
						"static frontend requires a nonempty index.html"
					);
				}
			}
			Unit::Service(name) => {
				ensure!(
					self.assets.is_none() && self.backend.is_some(),
					"service artifact shape differs"
				);
				if let Some(runtime) = &config.services[name].runtime {
					let path = runtime
						.container
						.image_pin
						.to_str()
						.context("toolchain pin path is not UTF-8")?;
					let pin = String::from_utf8(project.read_file(path, 4096)?)?;
					ensure!(
						self.toolchain_image.as_deref() == Some(pin.trim())
							&& image_pin(pin.trim()),
						"service toolchain image differs from its immutable project pin"
					);
				} else {
					ensure!(
						self.toolchain_image.is_none(),
						"undeclared service toolchain image"
					);
				}
			}
		}
		Ok(())
	}

	pub(super) fn check(&self, root: &Root) -> Result<()> {
		let mut expected = Vec::new();
		if let Some(tree) = &self.assets {
			expected.push("assets".to_owned());
			ensure!(
				root.beneath("assets")?.tree(&[])? == *tree,
				"graph assets differ from pinned membership, modes or bytes"
			);
		}
		if let Some(backend) = &self.backend {
			expected.push("backend".to_owned());
			ensure!(
				root.executable("backend")? == *backend,
				"graph backend differs from pinned mode or bytes"
			);
		}
		if let Some(service) = &self.service {
			expected.push("service".to_owned());
			ensure!(
				root.beneath("service")?.tree(&[])? == service.inputs,
				"service evidence differs from pinned bytes, membership or modes"
			);
		}
		ensure!(
			root.names()? == expected,
			"graph unit contains missing or undeclared files"
		);
		Ok(())
	}
}

pub(super) fn unit(value: &str) -> Result<Unit> {
	let unit = match value.split_once('.') {
		Some(("frontends", name)) => Unit::Frontend(name.into()),
		Some(("services", name)) => Unit::Service(name.into()),
		_ => anyhow::bail!("invalid qualified graph unit"),
	};
	Ok(unit)
}

fn image_pin(value: &str) -> bool {
	value.rsplit_once("@sha256:").is_some_and(|(name, digest)| {
		!name.is_empty()
			&& name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
			&& crate::verify::digest_is_valid(digest)
	})
}
