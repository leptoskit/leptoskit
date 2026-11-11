//! Build exactly the selected graph units and capture their local artifacts.
//! This does not reproduce source, package runtime images or authorize
//! releases.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

mod capture;
mod services;

pub(crate) fn run(
	root: &Path,
	config: &LeptoskitConfig,
	units: &[String],
	release: bool,
	out: &Path,
) -> Result<PathBuf> {
	let selection = capture::selection(config, units)?;
	let mut builder = services::Builder::prepare(root, config, &selection, release)?;
	let output = capture::execute(root, config, &selection, release, out, |name| {
		builder.build(name)
	})?;
	println!(
		"Graph build captured at {}. Source reproduction, image packaging and deployment remain \
		 unchecked.",
		output.display()
	);
	Ok(output)
}
