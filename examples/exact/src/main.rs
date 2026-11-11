//! Prints an Exact example the way a screen reader would read it.
//!
//! ```sh
//! cargo run -p exact_examples              # the names
//! cargo run -p exact_examples -- caltrain  # that screen
//! ```

use std::process::ExitCode;

use exact_examples::EXAMPLES;
use exact_examples::find;
use leptoskit_testing::WidgetTester;

fn main() -> ExitCode {
	let mut args = std::env::args().skip(1);
	match args.next().as_deref() {
		None | Some("list") => {
			list();
			ExitCode::SUCCESS
		}
		Some("summary") => show(args.next().as_deref()),
		Some(name) => show(Some(name)),
	}
}

fn list() {
	println!("Exact examples rebuilt with leptoskit:\n");
	for example in EXAMPLES {
		println!("  {:<18} {}", example.name, example.teaches);
	}
	println!("\ncargo run -p exact_examples -- <name>\ncd examples/exact && leptoskit dev");
}

fn show(name: Option<&str>) -> ExitCode {
	let Some(name) = name else {
		eprintln!("summary needs an example name");
		return ExitCode::FAILURE;
	};
	let Some(example) = find(name) else {
		eprintln!("unknown example `{name}`");
		list();
		return ExitCode::FAILURE;
	};
	let render = example.render;
	match WidgetTester::new(render, 390.0, 844.0) {
		Ok(tester) => {
			println!("{}", tester.semantics());
			ExitCode::SUCCESS
		}
		Err(error) => {
			eprintln!("{error}");
			ExitCode::FAILURE
		}
	}
}
