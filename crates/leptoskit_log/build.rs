//! Compiles the `os_log` shim on Apple targets. `os_log` is a C macro that
//! stores its format string in the image, so Rust reaches it through one C
//! function rather than re-encoding the macro's argument buffer.

fn main() {
	println!("cargo::rerun-if-changed=apple/os_log.c");

	if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
		cc::Build::new()
			.file("apple/os_log.c")
			.compile("leptoskit_os_log");
	}
}
