//! Bounded ELF inspection. Never invokes a loader, `ldd`, or the artifact.
//! Format compatibility does not establish installed libraries or readiness.
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use serde::Serialize;

/// Architectures supported by the initial Linux release adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
	#[value(name = "x86_64")]
	X86_64,
	Aarch64,
}

#[derive(Debug, Serialize)]
pub(crate) struct Format {
	pub architecture: Architecture,
	pub interpreter: Option<String>,
	pub needed: Vec<String>,
}

struct Segment {
	kind: u32,
	flags: u32,
	offset: u64,
	address: u64,
	size: u64,
	memory: u64,
}

fn bytes<const N: usize>(file: &mut std::fs::File, offset: u64) -> Result<[u8; N]> {
	file.seek(SeekFrom::Start(offset))?;
	let mut result = [0; N];
	file.read_exact(&mut result)?;
	Ok(result)
}
fn word(data: &[u8], offset: usize) -> u16 {
	u16::from_le_bytes([data[offset], data[offset + 1]])
}
fn dword(data: &[u8], offset: usize) -> u32 {
	u32::from_le_bytes(
		data[offset..offset + 4]
			.try_into()
			.expect("fixed ELF field"),
	)
}
fn qword(data: &[u8], offset: usize) -> u64 {
	u64::from_le_bytes(
		data[offset..offset + 8]
			.try_into()
			.expect("fixed ELF field"),
	)
}
fn extent(offset: u64, size: u64, limit: u64) -> Result<()> {
	ensure!(
		offset.checked_add(size).is_some_and(|end| end <= limit),
		"ELF range exceeds file"
	);
	Ok(())
}
fn text(data: &[u8]) -> Result<String> {
	let end = data
		.iter()
		.position(|b| *b == 0)
		.context("unterminated ELF string")?;
	let value = std::str::from_utf8(&data[..end])?;
	ensure!(
		!value.is_empty() && !value.chars().any(char::is_control),
		"invalid ELF string"
	);
	Ok(value.into())
}

pub(crate) fn inspect(mut file: std::fs::File, expected: Architecture) -> Result<Format> {
	let length = file.metadata()?.len();
	ensure!(
		length <= crate::verify::assets::MAX_BYTES,
		"executable exceeds artifact budget"
	);
	let header = bytes::<64>(&mut file, 0)?;
	ensure!(
		&header[..7] == b"\x7fELF\x02\x01\x01",
		"requires 64-bit little-endian ELF; build graph artifacts on the target Linux \
		 architecture with lk build"
	);
	ensure!(
		matches!(header[7], 0 | 3) && header[8] == 0,
		"unsupported ELF ABI"
	);
	ensure!(
		matches!(word(&header, 16), 2 | 3) && dword(&header, 20) == 1 && word(&header, 52) == 64,
		"invalid ELF executable header"
	);
	let architecture = match word(&header, 18) {
		62 => Architecture::X86_64,
		183 => Architecture::Aarch64,
		_ => anyhow::bail!("unsupported ELF machine"),
	};
	ensure!(
		architecture == expected,
		"executable architecture differs from named Linux host"
	);
	let offset = qword(&header, 32);
	let count = u64::from(word(&header, 56));
	ensure!(
		word(&header, 54) == 56 && (1..=1024).contains(&count),
		"invalid ELF program header table"
	);
	extent(offset, count * 56, length)?;
	let mut segments = Vec::new();
	for index in 0..count {
		let raw = bytes::<56>(&mut file, offset + index * 56)?;
		let segment = Segment {
			kind: dword(&raw, 0),
			flags: dword(&raw, 4),
			offset: qword(&raw, 8),
			address: qword(&raw, 16),
			size: qword(&raw, 32),
			memory: qword(&raw, 40),
		};
		extent(segment.offset, segment.size, length)?;
		if segment.kind == 1 {
			ensure!(
				segment.size <= segment.memory
					&& segment.address.checked_add(segment.memory).is_some(),
				"invalid ELF load segment"
			);
			ensure!(
				segment.flags & 3 != 3,
				"writable executable ELF segment is unsupported"
			);
		}
		ensure!(
			segment.kind != 0x6474_e551 || segment.flags & 1 == 0,
			"executable stack is unsupported"
		);
		segments.push(segment);
	}
	let entry = qword(&header, 24);
	ensure!(
		segments.iter().any(|s| {
			s.kind == 1 && s.flags & 1 != 0 && entry >= s.address && entry - s.address < s.size
		}),
		"ELF entry is not in executable file-backed memory"
	);
	let mut interpreter = None;
	for segment in segments.iter().filter(|s| s.kind == 3) {
		ensure!(
			interpreter.is_none() && (2..=4096).contains(&segment.size),
			"invalid ELF interpreter"
		);
		let mut raw = vec![0; usize::try_from(segment.size)?];
		file.seek(SeekFrom::Start(segment.offset))?;
		file.read_exact(&mut raw)?;
		let path = text(&raw)?;
		ensure!(
			path.starts_with('/')
				&& !path.split('/').any(|p| p == ".." || p == ".")
				&& raw.len() == path.len() + 1,
			"unsafe ELF interpreter"
		);
		interpreter = Some(path);
	}
	let needed = libraries(&mut file, &segments)?;
	ensure!(
		needed.is_empty() || interpreter.is_some(),
		"dynamic executable has no interpreter"
	);
	Ok(Format {
		architecture,
		interpreter,
		needed,
	})
}

fn libraries(file: &mut std::fs::File, segments: &[Segment]) -> Result<Vec<String>> {
	let dynamic: Vec<_> = segments.iter().filter(|s| s.kind == 2).collect();
	ensure!(dynamic.len() <= 1, "duplicate ELF dynamic segment");
	let Some(dynamic) = dynamic.first() else {
		return Ok(Vec::new());
	};
	ensure!(
		dynamic.size <= 65536 && dynamic.size % 16 == 0,
		"invalid ELF dynamic table"
	);
	let mut table = None;
	let mut size = None;
	let mut offsets = Vec::new();
	let mut terminated = false;
	for index in 0..dynamic.size / 16 {
		let raw = bytes::<16>(file, dynamic.offset + index * 16)?;
		let tag = qword(&raw, 0);
		let value = qword(&raw, 8);
		match tag {
			0 => {
				terminated = true;
				break;
			}
			1 => offsets.push(value),
			5 => {
				ensure!(table.replace(value).is_none(), "duplicate ELF string table");
			}
			10 => {
				ensure!(size.replace(value).is_none(), "duplicate ELF string size");
			}
			15 | 29 => {
				anyhow::bail!(
					"ELF RPATH/RUNPATH requires an explicit library installation contract"
				)
			}
			_ => {}
		}
	}
	ensure!(terminated, "unterminated ELF dynamic table");
	if offsets.is_empty() {
		return Ok(Vec::new());
	}
	let address = table.context("missing ELF string table")?;
	let size = size.context("missing ELF string size")?;
	ensure!(size <= 1024 * 1024, "ELF strings exceed inspection budget");
	let load = segments
		.iter()
		.find(|s| {
			s.kind == 1
				&& address >= s.address
				&& address - s.address <= s.size
				&& size <= s.size - (address - s.address)
		})
		.context("ELF strings outside file-backed load segment")?;
	let mut strings = vec![0; usize::try_from(size)?];
	file.seek(SeekFrom::Start(load.offset + address - load.address))?;
	file.read_exact(&mut strings)?;
	offsets
		.into_iter()
		.map(|offset| {
			let offset = usize::try_from(offset)?;
			let name = text(
				strings
					.get(offset..)
					.context("invalid ELF dependency string offset")?,
			)?;
			ensure!(
				!name.contains('/'),
				"ELF library paths require explicit installation"
			);
			Ok(name)
		})
		.collect()
}

#[cfg(test)]
mod tests;
