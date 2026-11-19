//! The source archive: what goes in, determinism, the bounds, secrets, and
//! unpacking hostile archives.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use std::time::SystemTime;

use flate2::Compression;
use flate2::write::GzEncoder;
use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::archive::Limits;
use leptoskit_cloud::archive::PackError;
use leptoskit_cloud::archive::PackOptions;
use leptoskit_cloud::archive::Packed;
use leptoskit_cloud::archive::UnpackError;
use leptoskit_cloud::archive::pack;
use leptoskit_cloud::archive::unpack;
use tempfile::TempDir;

fn write(root: &Path, path: &str, contents: &str) {
	let full = root.join(path);
	fs::create_dir_all(full.parent().unwrap()).unwrap();
	fs::write(full, contents).unwrap();
}

/// A project with files to keep, files `.gitignore` and the always-excluded
/// directories leave out, an executable, and a symlink inside the root.
fn project() -> TempDir {
	let root = tempfile::tempdir().unwrap();
	let path = root.path();
	write(path, "Cargo.toml", "[package]\nname = \"notes\"\n");
	write(path, "leptoskit.toml", "[cloud]\nproject = \"notes\"\n");
	write(path, "src/main.rs", "fn main() {}\n");
	write(path, "src/lib.rs", "pub fn notes() {}\n");
	write(path, ".gitignore", "ignored.txt\n/cache/\n");
	write(path, "ignored.txt", "left out\n");
	write(path, "cache/blob", "left out\n");
	write(path, "target/debug/notes", "left out\n");
	write(path, "crates/inner/target/x", "left out\n");
	write(path, "node_modules/x/index.js", "left out\n");
	write(path, "dist/app.js", "left out\n");
	write(path, ".leptoskit/db/data", "left out\n");
	write(path, "scripts/build.sh", "#!/bin/sh\n");
	fs::set_permissions(
		path.join("scripts/build.sh"),
		fs::Permissions::from_mode(0o750),
	)
	.unwrap();
	symlink("src/lib.rs", path.join("lib.rs")).unwrap();
	root
}

const KEPT: [&str; 7] = [
	".gitignore",
	"Cargo.toml",
	"leptoskit.toml",
	"lib.rs",
	"scripts/build.sh",
	"src/lib.rs",
	"src/main.rs",
];

fn walk() -> PackOptions {
	PackOptions {
		use_git: false,
		..PackOptions::default()
	}
}

fn packed(root: &Path, options: &PackOptions) -> Packed {
	pack(root, options).unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn a_walk_keeps_the_project_and_leaves_out_ignored_and_built_files() {
	let root = project();
	let archive = packed(root.path(), &walk());
	assert_eq!(archive.files, KEPT);
	assert_eq!(archive.digest, Digest::of(&archive.bytes));
	let expected_bytes: u64 = KEPT
		.iter()
		.filter(|path| **path != "lib.rs")
		.map(|path| fs::metadata(root.path().join(path)).unwrap().len())
		.sum();
	assert_eq!(archive.uncompressed_bytes, expected_bytes);
}

#[test]
fn packing_is_deterministic() {
	let root = project();
	let first = packed(root.path(), &walk());
	let second = packed(root.path(), &walk());
	assert_eq!(first.bytes, second.bytes);
	assert_eq!(first.digest, second.digest);

	// Times and non-executable permission bits don't reach the archive.
	let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
	fs::File::options()
		.write(true)
		.open(root.path().join("src/main.rs"))
		.unwrap()
		.set_modified(old)
		.unwrap();
	fs::set_permissions(
		root.path().join("Cargo.toml"),
		fs::Permissions::from_mode(0o600),
	)
	.unwrap();
	fs::set_permissions(
		root.path().join("scripts/build.sh"),
		fs::Permissions::from_mode(0o700),
	)
	.unwrap();
	let third = packed(root.path(), &walk());
	assert_eq!(first.bytes, third.bytes);

	write(root.path(), "src/main.rs", "fn main() { }\n");
	assert_ne!(packed(root.path(), &walk()).digest, first.digest);
}

#[test]
fn unpacking_restores_files_modes_and_links() {
	let root = project();
	let archive = packed(root.path(), &walk());
	let destination = tempfile::tempdir().unwrap();
	let out = destination.path().join("source");
	let unpacked = unpack(archive.bytes.as_slice(), &out, &Limits::DEFAULT).unwrap();
	assert_eq!(unpacked.files, KEPT);
	assert_eq!(unpacked.bytes, archive.uncompressed_bytes);

	for path in KEPT {
		assert_eq!(
			fs::read(out.join(path)).unwrap(),
			fs::read(root.path().join(path)).unwrap(),
			"{path}"
		);
	}

	let mode = |path: &str| {
		fs::symlink_metadata(out.join(path))
			.unwrap()
			.permissions()
			.mode()
			& 0o7777
	};
	assert_eq!(mode("scripts/build.sh"), 0o755);
	assert_eq!(mode("src/main.rs"), 0o644);
	assert_eq!(
		fs::read_link(out.join("lib.rs")).unwrap(),
		Path::new("src/lib.rs")
	);
	assert!(!out.join("ignored.txt").exists());
}

fn git(root: &Path, args: &[&str]) {
	let mut command = Command::new("git");
	command.arg("-C").arg(root).args(args);

	// The pre-push hook runs these tests with the hook's repository in
	// the environment.
	for variable in [
		"GIT_DIR",
		"GIT_WORK_TREE",
		"GIT_INDEX_FILE",
		"GIT_COMMON_DIR",
	] {
		command.env_remove(variable);
	}

	let output = command.output().expect("git is installed");
	assert!(
		output.status.success(),
		"git {args:?}: {}",
		String::from_utf8_lossy(&output.stderr)
	);
}

#[test]
fn in_a_checkout_git_chooses_the_files() {
	let root = project();
	let path = root.path();
	git(path, &["init", "--quiet"]);
	write(path, "tracked-then-deleted.rs", "gone\n");
	git(path, &["add", "--all"]);
	fs::remove_file(path.join("tracked-then-deleted.rs")).unwrap();
	write(path, "untracked.rs", "kept\n");
	// Git keeps a tracked file even when `.gitignore` names it later.
	write(path, ".gitignore", "ignored.txt\n/cache/\nsrc/main.rs\n");

	let archive = packed(path, &PackOptions::default());
	let mut expected: Vec<&str> = KEPT.to_vec();
	expected.push("untracked.rs");
	expected.sort_unstable();
	assert_eq!(archive.files, expected);

	// A subdirectory of a checkout packs only what is under it.
	let archive = packed(&path.join("src"), &PackOptions::default());
	assert_eq!(archive.files, ["lib.rs", "main.rs"]);
}

#[test]
fn leptoskitignore_leaves_out_more() {
	let root = project();
	write(
		root.path(),
		".leptoskitignore",
		"scripts/\n*.toml\n!leptoskit.toml\n",
	);
	let archive = packed(root.path(), &walk());
	assert_eq!(
		archive.files,
		[
			".gitignore",
			".leptoskitignore",
			"leptoskit.toml",
			"lib.rs",
			"src/lib.rs",
			"src/main.rs",
		]
	);
}

#[test]
fn secret_files_are_refused_unless_ignored() {
	let root = project();
	write(root.path(), ".env", "TOKEN=secret\n");
	write(root.path(), "keys/AuthKey.p8", "key\n");
	let error = pack(root.path(), &walk()).unwrap_err();
	assert!(
		matches!(&error, PackError::Secrets { paths } if paths == &[".env", "keys/AuthKey.p8"])
	);
	let message = error.to_string();
	assert!(message.contains("`.env`, `keys/AuthKey.p8`"), "{message}");
	assert!(message.contains("lk cloud secret set"), "{message}");
	assert!(!message.contains("TOKEN"), "{message}");

	write(root.path(), ".leptoskitignore", "keys/\n");
	write(root.path(), ".gitignore", "ignored.txt\n/cache/\n.env\n");
	let archive = packed(root.path(), &walk());
	assert!(
		!archive
			.files
			.iter()
			.any(|path| path.contains("env") || path.contains("keys"))
	);
}

#[test]
fn links_that_leave_the_project_are_refused() {
	for target in ["../outside", "/etc/hosts", "src/../../outside"] {
		let root = project();
		symlink(target, root.path().join("escape")).unwrap();
		let error = pack(root.path(), &walk()).unwrap_err();
		assert!(
			matches!(&error, PackError::EscapingLink { path, target: found } if path == "escape" && found == target),
			"{target}: {error}"
		);
	}
}

#[test]
fn the_bounds_are_held_and_name_the_file() {
	let root = tempfile::tempdir().unwrap();
	write(root.path(), "a.txt", "0123456789");
	write(root.path(), "b.txt", "0123456789");
	write(root.path(), "c.txt", "0123456789A");
	let limits = |max_files, max_bytes, max_file_bytes| {
		PackOptions {
			limits: Limits {
				max_files,
				max_bytes,
				max_file_bytes,
			},
			use_git: false,
		}
	};

	let error = pack(root.path(), &limits(2, 1000, 1000)).unwrap_err();
	assert!(
		matches!(error, PackError::TooManyFiles { limit: 2 }),
		"{error}"
	);

	let error = pack(root.path(), &limits(10, 1000, 10)).unwrap_err();
	assert!(
		matches!(&error, PackError::FileTooLarge { path, bytes: 11, limit: 10 } if path == "c.txt"),
		"{error}"
	);
	assert!(error.to_string().contains("`c.txt`"), "{error}");

	let error = pack(root.path(), &limits(10, 25, 100)).unwrap_err();
	assert!(
		matches!(error, PackError::TooLarge { limit: 25 }),
		"{error}"
	);

	assert_eq!(
		packed(root.path(), &limits(3, 31, 11)).uncompressed_bytes,
		31
	);
}

/// One entry of a hand-built archive, written without the checks
/// `tar::Builder` makes, as a hostile client would.
enum Raw<'a> {
	File(&'a str, &'a [u8]),
	Directory(&'a str),
	Symlink(&'a str, &'a str),
	HardLink(&'a str, &'a str),
	Fifo(&'a str),
	/// A header that says `size` bytes and carries `data`.
	Sized(&'a str, u64, &'a [u8]),
}

fn hostile(entries: &[Raw<'_>]) -> Vec<u8> {
	let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));

	for entry in entries {
		let mut header = tar::Header::new_ustar();
		let (path, link, kind, data): (&str, &str, tar::EntryType, &[u8]) = match *entry {
			Raw::File(path, data) | Raw::Sized(path, _, data) => {
				(path, "", tar::EntryType::Regular, data)
			}
			Raw::Directory(path) => (path, "", tar::EntryType::Directory, &[]),
			Raw::Symlink(path, target) => (path, target, tar::EntryType::Symlink, &[]),
			Raw::HardLink(path, target) => (path, target, tar::EntryType::Link, &[]),
			Raw::Fifo(path) => (path, "", tar::EntryType::Fifo, &[]),
		};
		let old = header.as_old_mut();
		old.name.fill(0);
		old.name[..path.len()].copy_from_slice(path.as_bytes());
		old.linkname.fill(0);
		old.linkname[..link.len()].copy_from_slice(link.as_bytes());
		header.set_entry_type(kind);
		header.set_mode(0o644);
		header.set_size(match *entry {
			Raw::Sized(_, size, _) => size,
			_ => data.len() as u64,
		});
		header.set_cksum();
		builder.append(&header, data).unwrap();
	}

	builder.into_inner().unwrap().finish().unwrap()
}

/// Unpacks `entries` into a fresh directory inside a parent that a
/// successful escape would write to.
fn unpack_hostile(entries: &[Raw<'_>], limits: &Limits) -> (Result<(), UnpackError>, TempDir) {
	let parent = tempfile::tempdir().unwrap();
	let bytes = hostile(entries);
	let result = unpack(bytes.as_slice(), &parent.path().join("out"), limits).map(|_| ());
	(result, parent)
}

/// Whatever happened, nothing appeared beside the destination.
fn assert_contained(parent: &TempDir) {
	let names: Vec<String> = fs::read_dir(parent.path())
		.unwrap()
		.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
		.collect();
	assert_eq!(
		names,
		["out"],
		"something was written outside the destination"
	);
}

#[test]
fn hostile_paths_are_refused() {
	for path in [
		"../evil",
		"/tmp/evil",
		"a/../../evil",
		"a//b",
		"./a",
		"a\\b",
	] {
		let (result, parent) = unpack_hostile(&[Raw::File(path, b"x")], &Limits::DEFAULT);
		assert!(
			matches!(&result, Err(UnpackError::Path { path: found, .. }) if found == path),
			"{path}: {result:?}"
		);
		assert_contained(&parent);
	}
}

#[test]
fn links_that_leave_the_destination_are_refused() {
	let cases: [&[Raw<'_>]; 5] = [
		&[Raw::Symlink("escape", "../evil")],
		&[Raw::Symlink("escape", "/etc/passwd")],
		&[Raw::Symlink("deep/escape", "../../evil")],
		// Lexically `sub/evil`, but `sub/up` is the root, so `..` leaves it.
		&[
			Raw::Symlink("sub/up", ".."),
			Raw::Symlink("escape", "sub/up/../evil"),
		],
		&[
			Raw::Symlink("escape", "loop"),
			Raw::Symlink("loop", "escape"),
		],
	];

	for entries in cases {
		let (result, parent) = unpack_hostile(entries, &Limits::DEFAULT);
		assert!(
			matches!(result, Err(UnpackError::EscapingLink { .. })),
			"{result:?}"
		);
		assert_contained(&parent);
		assert!(
			fs::symlink_metadata(parent.path().join("out/escape")).is_err(),
			"no link is created when any is refused"
		);
	}
}

#[test]
fn nothing_is_written_through_a_link() {
	let (result, parent) = unpack_hostile(
		&[Raw::Symlink("dir", "inside"), Raw::File("dir/evil", b"x")],
		&Limits::DEFAULT,
	);
	assert!(
		matches!(&result, Err(UnpackError::ThroughLink { path, link }) if path == "dir/evil" && link == "dir"),
		"{result:?}"
	);
	assert_contained(&parent);

	let (result, _parent) = unpack_hostile(
		&[Raw::File("a/b", b"x"), Raw::Symlink("a", "elsewhere")],
		&Limits::DEFAULT,
	);
	assert!(
		matches!(result, Err(UnpackError::Collision(_))),
		"{result:?}"
	);

	let (result, _parent) = unpack_hostile(
		&[Raw::File("a", b"x"), Raw::File("a", b"y")],
		&Limits::DEFAULT,
	);
	assert!(
		matches!(result, Err(UnpackError::Collision(_))),
		"{result:?}"
	);

	let (result, _parent) = unpack_hostile(
		&[Raw::File("a", b"x"), Raw::File("a/b", b"y")],
		&Limits::DEFAULT,
	);
	assert!(
		matches!(result, Err(UnpackError::Collision(_))),
		"{result:?}"
	);
}

#[test]
fn only_files_directories_and_symlinks_are_accepted() {
	for entries in [
		&[Raw::File("target", b"x"), Raw::HardLink("hard", "target")][..],
		&[Raw::Fifo("pipe")][..],
	] {
		let (result, parent) = unpack_hostile(entries, &Limits::DEFAULT);
		assert!(
			matches!(result, Err(UnpackError::Unsupported { .. })),
			"{result:?}"
		);
		assert_contained(&parent);
	}

	let (result, parent) = unpack_hostile(
		&[
			Raw::Directory("docs/"),
			Raw::File("docs/a.md", b"a"),
			Raw::Symlink("readme.md", "docs/a.md"),
		],
		&Limits::DEFAULT,
	);
	result.unwrap();
	assert_eq!(
		fs::read_to_string(parent.path().join("out/readme.md")).unwrap(),
		"a"
	);
}

#[test]
fn the_bounds_are_read_from_the_headers() {
	let limits = Limits {
		max_files: 2,
		max_bytes: 10,
		max_file_bytes: 6,
	};

	let (result, parent) = unpack_hostile(&[Raw::Sized("huge", 1 << 40, b"")], &limits);
	assert!(
		matches!(&result, Err(UnpackError::FileTooLarge { path, bytes, .. }) if path == "huge" && *bytes == 1 << 40),
		"{result:?}"
	);
	assert!(
		!parent.path().join("out/huge").exists(),
		"refused before writing"
	);

	let (result, _parent) = unpack_hostile(
		&[Raw::File("a", b"123456"), Raw::File("b", b"123456")],
		&limits,
	);
	assert!(
		matches!(result, Err(UnpackError::TooLarge { limit: 10 })),
		"{result:?}"
	);

	let (result, _parent) = unpack_hostile(
		&[
			Raw::File("a", b"1"),
			Raw::File("b", b"2"),
			Raw::File("c", b"3"),
		],
		&limits,
	);
	assert!(
		matches!(result, Err(UnpackError::TooManyFiles { limit: 2 })),
		"{result:?}"
	);
}

#[test]
fn the_destination_must_be_empty() {
	let parent = tempfile::tempdir().unwrap();
	write(parent.path(), "out/existing", "x");
	let bytes = hostile(&[Raw::File("a", b"x")]);
	let result = unpack(
		bytes.as_slice(),
		&parent.path().join("out"),
		&Limits::DEFAULT,
	);
	assert!(
		matches!(result, Err(UnpackError::DestinationNotEmpty(_))),
		"{result:?}"
	);
	assert!(!parent.path().join("out/a").exists());
}
