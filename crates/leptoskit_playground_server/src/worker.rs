//! Fixed worker entrypoint inside the untrusted compilation container.
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use base64::Engine as _;
use leptoskit_playground_server::Compilation;
use leptoskit_playground_server::diagnostics;
use leptoskit_playground_server::raw_diagnostics;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "reason")]
enum CargoMessage {
	#[serde(rename = "compiler-artifact")]
	Artifact {
		target: CargoTarget,
		filenames: Vec<PathBuf>,
	},
	#[serde(other)]
	Other,
}

impl CargoMessage {
	fn artifact(&self) -> Option<(&CargoTarget, &[PathBuf])> {
		match self {
			Self::Artifact { target, filenames } => Some((target, filenames)),
			Self::Other => None,
		}
	}
}

#[derive(Deserialize)]
struct CargoTarget {
	name: String,
}

fn is_wasm_library(path: &Path) -> bool {
	path.extension()
		.is_some_and(|extension| extension == "rlib" || extension == "rmeta")
		&& path
			.components()
			.any(|part| part.as_os_str() == "wasm32-unknown-unknown")
}

fn main() -> anyhow::Result<()> {
	if std::env::args().nth(1).as_deref() == Some("--probe-network") {
		let address = "1.1.1.1:443".parse()?;
		anyhow::ensure!(
			std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2))
				.is_err(),
			"network unexpectedly available"
		);
		println!("network denied");

		return Ok(());
	}

	let mut source = String::new();
	// The HTTP boundary applies PLAY_REQUEST_BYTES before starting a worker.
	std::io::stdin().read_to_string(&mut source)?;
	std::fs::write("/work/user.rs", source)?;
	std::fs::copy("/harness/src/lib.rs", "/work/lib.rs")?;
	std::io::stdout().write_all(&serde_json::to_vec(&compile()?)?)?;
	Ok(())
}

/// Select the immutable dependency artifacts without assuming Cargo's layout.
fn dependencies(rustc: &mut Command) -> anyhow::Result<()> {
	let manifest = std::fs::read_to_string("/harness/artifacts.json")?;
	let artifacts = manifest
		.lines()
		.map(serde_json::from_str::<CargoMessage>)
		.collect::<Result<Vec<_>, _>>()?;
	let mut directories = std::collections::BTreeSet::new();
	let mut libraries = std::collections::BTreeMap::<&str, Vec<&Path>>::new();

	for (target, filenames) in artifacts.iter().filter_map(CargoMessage::artifact) {
		for path in filenames {
			if let Some(parent) = path.parent() {
				directories.insert(parent);
			}

			if is_wasm_library(path) {
				libraries.entry(&target.name).or_default().push(path);
			}
		}
	}

	for directory in directories {
		rustc
			.arg("-L")
			.arg(format!("dependency={}", directory.display()));
	}

	for name in ["leptoskit", "wasm_bindgen", "serde"] {
		let files = libraries
			.get(name)
			.ok_or_else(|| anyhow::anyhow!("missing precompiled {name}"))?;

		for file in files {
			rustc
				.arg("--extern")
				.arg(format!("{name}={}", file.display()));
		}
	}

	Ok(())
}

/// Compile the reader's crate with the same latency profile as its
/// dependencies.
fn compiler() -> anyhow::Result<Command> {
	let mut rustc = Command::new("rustc");
	rustc
		.env("CARGO_PKG_NAME", "playground")
		.env("CARGO_PKG_VERSION", "0.0.0")
		.env("CARGO_MANIFEST_DIR", "/work");
	rustc.args([
		"/work/lib.rs",
		"--edition=2024",
		"--crate-name=playground",
		"--crate-type=cdylib",
		"--target=wasm32-unknown-unknown",
		"-Copt-level=0",
		"-Cdebuginfo=0",
		"-Ccodegen-units=256",
		"--error-format=json",
		"--json=diagnostic-rendered-ansi",
		"-o",
		"/work/playground.wasm",
	]);
	dependencies(&mut rustc)?;
	Ok(rustc)
}

/// Return mapped diagnostics or the complete browser bundle.
fn compile() -> anyhow::Result<Compilation> {
	let output = compiler()?.output()?;
	let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
	let messages = raw_diagnostics(&stderr);
	let mut result = Compilation {
		diagnostics: diagnostics(&stderr),
		stderr: messages
			.iter()
			.filter_map(|value| value["rendered"].as_str())
			.collect::<String>(),
		raw_diagnostics: messages,
		..Default::default()
	};

	if output.status.success() {
		let output = Command::new("wasm-bindgen")
			.args([
				"/work/playground.wasm",
				"--target=web",
				"--out-dir=/work/pkg",
				"--out-name=playground",
			])
			.output()?;
		anyhow::ensure!(
			output.status.success(),
			"wasm-bindgen failed: {}",
			String::from_utf8_lossy(&output.stderr)
		);
		result.js = std::fs::read_to_string("/work/pkg/playground.js")?;
		result.wasm = base64::engine::general_purpose::STANDARD
			.encode(std::fs::read("/work/pkg/playground_bg.wasm")?);
	}

	Ok(result)
}
