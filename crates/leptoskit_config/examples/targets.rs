//! Inspect named targets without compiling the application.

use leptoskit_config::LeptoskitConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let root = std::env::args_os().nth(1).ok_or("expected project root")?;
	let config = LeptoskitConfig::load_from(std::path::Path::new(&root))?;
	for (name, target) in &config.targets {
		if let Some(reason) = &target.unavailable {
			println!("{name}: unavailable: {reason}");
		} else {
			let selected = config.resolve_target(Some(name))?;
			println!(
				"{name}: {} client={:?} server={} output={}",
				target.platform,
				selected.build.client_package,
				selected.server_package(),
				selected.output_directory("dist").display()
			);
		}
	}
	Ok(())
}
