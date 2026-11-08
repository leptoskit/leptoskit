//! Writes the test suite's skinned character to a `.glb`, the model
//! `examples/scene3d` shows.
//!
//! ```sh
//! cargo run -p leptoskit_3d --example character -- out.glb [joints]
//! ```

#[path = "../tests/support/fixture.rs"]
mod fixture;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut arguments = std::env::args().skip(1);
	let Some(output) = arguments.next() else {
		eprintln!("usage: character out.glb [joints]");
		std::process::exit(2);
	};
	let joints: usize = arguments.next().map_or(Ok(8), |value| value.parse())?;
	let bytes = fixture::character(joints.clamp(2, 64));
	std::fs::write(&output, &bytes)?;
	println!("{output}: {} bytes", bytes.len());
	Ok(())
}
