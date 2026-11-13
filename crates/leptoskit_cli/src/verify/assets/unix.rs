use std::fs::File;
use std::io::Read as _;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use rustix::fs::AtFlags;
use rustix::fs::Dir;
use rustix::fs::FileType;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use sha2::Digest as _;
use sha2::Sha256;

use super::MAX_BYTES;
use super::MAX_DEPTH;
use super::MAX_ENTRIES;
use super::Member;
use super::Root;
use super::Tree;

const DIRECTORY: OFlags = OFlags::RDONLY
	.union(OFlags::DIRECTORY)
	.union(OFlags::NOFOLLOW)
	.union(OFlags::CLOEXEC);

pub(super) fn open(path: &Path) -> Result<Root> {
	Ok(Root {
		file: rustix::fs::open(path, DIRECTORY, Mode::empty())?.into(),
	})
}

pub(super) fn beneath(root: &Root, relative: &str) -> Result<Root> {
	let mut file = root.file.try_clone()?;

	if !relative.is_empty() {
		for name in relative.split('/') {
			file = rustix::fs::openat(&file, name, DIRECTORY, Mode::empty())?.into();
		}
	}

	Ok(Root { file })
}

fn mode(file: &File) -> Result<u32> {
	Ok(file.metadata()?.permissions().mode() & 0o7777)
}

fn file_at(parent: &File, name: &str, required_mode: Option<u32>) -> Result<File> {
	let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
	let file: File = rustix::fs::openat(parent, name, flags, Mode::empty())?.into();
	ensure!(
		file.metadata()?.is_file(),
		"asset links and special files are unsupported"
	);

	if let Some(required) = required_mode {
		ensure!(
			mode(&file)? == required,
			"artifact file mode differs from its contract"
		);
	}

	Ok(file)
}

pub(super) fn open_executable(root: &Root, relative: &str) -> Result<File> {
	let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
	file_at(&root.beneath(parent)?.file, name, Some(0o755))
}

pub(super) fn tree(root: &Root, excluded: &[String]) -> Result<Tree> {
	let mut tree = Tree::new();
	let mut remaining = MAX_BYTES;
	walk(&root.file, "", excluded, &mut tree, &mut remaining, 0)?;
	super::validate(&tree)?;
	Ok(tree)
}

fn names(directory: &File) -> Result<Vec<String>> {
	let mut names = Vec::new();

	for entry in Dir::read_from(directory)? {
		let entry = entry?;
		let name = entry
			.file_name()
			.to_str()
			.context("asset names must be UTF-8")?;

		if matches!(name, "." | "..") {
			continue;
		}

		ensure!(
			names.len() < MAX_ENTRIES,
			"asset directory has too many entries"
		);
		names.push(name.to_owned());
	}

	names.sort();
	Ok(names)
}

pub(super) fn root_names(root: &Root) -> Result<Vec<String>> {
	names(&root.file)
}

fn walk(
	directory: &File,
	prefix: &str,
	excluded: &[String],
	tree: &mut Tree,
	remaining: &mut u64,
	depth: usize,
) -> Result<()> {
	ensure!(depth <= MAX_DEPTH, "asset tree is too deep");
	ensure!(
		mode(directory)? == 0o755,
		"asset directories require mode 0755"
	);

	for name in names(directory)? {
		let path = if prefix.is_empty() {
			name.clone()
		} else {
			format!("{prefix}/{name}")
		};

		ensure!(super::safe_path(&path), "unsafe asset path");
		if excluded.iter().any(|excluded| {
			path == *excluded
				|| path
					.strip_prefix(excluded)
					.is_some_and(|rest| rest.starts_with('/'))
		}) {
			continue;
		}

		ensure!(tree.len() < MAX_ENTRIES, "asset tree has too many entries");

		let stat = rustix::fs::statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)?;

		match FileType::from_raw_mode(stat.st_mode) {
			FileType::Directory => {
				let child: File =
					rustix::fs::openat(directory, name.as_str(), DIRECTORY, Mode::empty())?.into();
				tree.insert(
					path.clone(),
					Member::Directory {
						mode: mode(&child)?,
					},
				);
				walk(&child, &path, excluded, tree, remaining, depth + 1)?;
			}
			FileType::RegularFile => {
				let file = file_at(directory, &name, Some(0o644))?;
				let mut hash = Sha256::new();
				let size = std::io::copy(&mut file.take(*remaining + 1), &mut hash)?;
				ensure!(size <= *remaining, "asset tree exceeds its byte limit");
				*remaining -= size;
				tree.insert(
					path,
					Member::File {
						mode: 0o644,
						size,
						sha256: hex::encode(hash.finalize()),
					},
				);
			}
			_ => anyhow::bail!("asset links and special files are unsupported"),
		}
	}

	Ok(())
}

pub(super) fn copy(root: &Root, destination: &Path, expected: &Tree) -> Result<()> {
	std::fs::create_dir(destination)?;
	let dest = open(destination)?;
	rustix::fs::fchmod(&dest.file, Mode::from_raw_mode(0o755))?;

	for (path, member) in expected {
		let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
		let source_parent = root.beneath(parent)?;
		let dest_parent = dest.beneath(parent)?;

		match member {
			Member::Directory { .. } => {
				rustix::fs::mkdirat(&dest_parent.file, name, Mode::from_raw_mode(0o755))?;
				let child = dest.beneath(path)?;
				rustix::fs::fchmod(&child.file, Mode::from_raw_mode(0o755))?;
			}
			Member::File { size, sha256, .. } => {
				copy_file(
					&source_parent.file,
					&dest_parent.file,
					(name, name),
					*size,
					sha256,
					0o644,
				)?;
			}
		}
	}

	Ok(())
}

fn copy_file(
	source: &File,
	destination: &File,
	names: (&str, &str),
	size: u64,
	expected_hash: &str,
	file_mode: u32,
) -> Result<()> {
	let mut input = file_at(source, names.0, Some(file_mode))?.take(size + 1);

	let permissions = match file_mode {
		0o644 => Mode::from_raw_mode(0o644),
		0o755 => Mode::from_raw_mode(0o755),
		_ => anyhow::bail!("unsupported captured file mode"),
	};

	let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
	let mut output: File = rustix::fs::openat(destination, names.1, flags, permissions)?.into();
	let mut hash = Sha256::new();
	let mut written = 0_u64;
	let mut buffer = [0_u8; 16 * 1024];

	loop {
		let length = input.read(&mut buffer)?;

		if length == 0 {
			break;
		}

		written += length as u64;
		ensure!(written <= size, "asset grew while copying");
		hash.update(&buffer[..length]);
		output.write_all(&buffer[..length])?;
	}

	ensure!(
		written == size && hex::encode(hash.finalize()) == expected_hash,
		"asset bytes changed while copying"
	);
	rustix::fs::fchmod(&output, permissions)?;
	output.sync_all()?;
	Ok(())
}

pub(super) fn read_file(root: &Root, relative: &str, limit: u64) -> Result<Vec<u8>> {
	let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
	let parent = root.beneath(parent)?;
	let mut bytes = Vec::new();
	file_at(&parent.file, name, None)?
		.take(limit + 1)
		.read_to_end(&mut bytes)?;
	ensure!(
		bytes.len() as u64 <= limit,
		"artifact input exceeds its byte limit"
	);
	Ok(bytes)
}

pub(super) fn executable(root: &Root, relative: &str) -> Result<Member> {
	let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
	let parent = root.beneath(parent)?;
	let mut hash = Sha256::new();
	let size = std::io::copy(
		&mut file_at(&parent.file, name, Some(0o755))?.take(MAX_BYTES + 1),
		&mut hash,
	)?;
	ensure!(
		size > 0 && size <= MAX_BYTES,
		"executable is empty or too large"
	);
	let sha256 = hex::encode(hash.finalize());
	Ok(Member::File {
		mode: 0o755,
		size,
		sha256,
	})
}

pub(super) fn copy_executable(root: &Root, relative: &str, destination: &Path) -> Result<Member> {
	let member = executable(root, relative)?;
	let Member::File { size, sha256, .. } = &member else {
		unreachable!("executable returns a file")
	};
	let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
	let parent = root.beneath(parent)?;
	let dest_parent = open(
		destination
			.parent()
			.context("executable destination needs a directory")?,
	)?;
	let dest_name = destination
		.file_name()
		.and_then(|name| name.to_str())
		.context("invalid executable destination")?;

	copy_file(
		&parent.file,
		&dest_parent.file,
		(name, dest_name),
		*size,
		sha256,
		0o755,
	)?;
	Ok(member)
}
