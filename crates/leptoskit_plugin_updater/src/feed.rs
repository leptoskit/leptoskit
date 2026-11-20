//! Signed desktop release feed, semantic version comparison, staged rollout
//! gating, SHA-256 artifact verification, and [`FeedBackend`] for
//! [`crate::Updater`].
//!
//! Desktop apps ship native executables via signed update manifests rather than
//! Wasm code push. [`FeedBackend`] implements [`crate::Backend`] over any
//! host-supplied [`FeedTransport`] (for example HTTPS via `reqwest` or a custom
//! CDN client) and enforces HTTPS URLs, channel policies, semver ordering,
//! rollout eligibility, SHA-256 checksum verification, and optional signature
//! verification before staging an update.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::Write;

use crate::Backend;
use crate::Error;
use crate::Status;

/// Release stream a desktop app subscribes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UpdateChannel {
	/// Production stable channel.
	Stable,
	/// Pre-release beta channel.
	Beta,
	/// Nightly / canary channel.
	Canary,
	/// Custom channel name (ASCII letters, digits, `-`, `_`).
	Custom(String),
}

impl UpdateChannel {
	/// Wire/feed string representation.
	#[must_use]
	pub fn as_str(&self) -> &str {
		match self {
			Self::Stable => "stable",
			Self::Beta => "beta",
			Self::Canary => "canary",
			Self::Custom(name) => name,
		}
	}
}

/// Desktop target architecture key in a [`ReleaseManifest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetPlatformId {
	/// Apple Silicon macOS (`aarch64-apple-darwin`).
	MacosAarch64,
	/// Intel macOS (`x86_64-apple-darwin`).
	MacosX86_64,
	/// Universal macOS bundle (`universal-apple-darwin`).
	MacosUniversal,
	/// 64-bit x86 Windows (`x86_64-pc-windows-msvc`).
	WindowsX86_64,
	/// 64-bit ARM Windows (`aarch64-pc-windows-msvc`).
	WindowsAarch64,
	/// 64-bit x86 Linux (`x86_64-unknown-linux-gnu`).
	LinuxX86_64,
	/// 64-bit ARM Linux (`aarch64-unknown-linux-gnu`).
	LinuxAarch64,
}

impl TargetPlatformId {
	/// Canonical feed key name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::MacosAarch64 => "macos-aarch64",
			Self::MacosX86_64 => "macos-x86_64",
			Self::MacosUniversal => "macos-universal",
			Self::WindowsX86_64 => "windows-x86_64",
			Self::WindowsAarch64 => "windows-aarch64",
			Self::LinuxX86_64 => "linux-x86_64",
			Self::LinuxAarch64 => "linux-aarch64",
		}
	}

	/// Current compile-time desktop target, or `None` on non-desktop hosts.
	#[must_use]
	pub const fn current() -> Option<Self> {
		#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
		{
			Some(Self::MacosAarch64)
		}
		#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
		{
			Some(Self::MacosX86_64)
		}
		#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
		{
			Some(Self::WindowsX86_64)
		}
		#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
		{
			Some(Self::WindowsAarch64)
		}
		#[cfg(all(target_os = "linux", not(target_env = "ohos"), target_arch = "x86_64"))]
		{
			Some(Self::LinuxX86_64)
		}
		#[cfg(all(target_os = "linux", not(target_env = "ohos"), target_arch = "aarch64"))]
		{
			Some(Self::LinuxAarch64)
		}
		#[cfg(not(any(
			all(target_os = "macos", target_arch = "aarch64"),
			all(target_os = "macos", target_arch = "x86_64"),
			all(target_os = "windows", target_arch = "x86_64"),
			all(target_os = "windows", target_arch = "aarch64"),
			all(target_os = "linux", not(target_env = "ohos"), target_arch = "x86_64"),
			all(target_os = "linux", not(target_env = "ohos"), target_arch = "aarch64"),
		)))]
		{
			None
		}
	}
}

/// Parsed semantic version (`MAJOR.MINOR.PATCH[-PRERELEASE]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
	/// Major version number.
	pub major: u64,
	/// Minor version number.
	pub minor: u64,
	/// Patch version number.
	pub patch: u64,
	/// Optional pre-release label (for example `"beta.1"`).
	pub pre: Option<String>,
}

impl SemVer {
	/// Parses a `MAJOR.MINOR.PATCH[-PRE][+BUILD]` string (ignoring build
	/// metadata per `SemVer` 2.0.0 precedence).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] when the string is not valid
	/// `SemVer`.
	pub fn parse(input: &str) -> Result<Self, Error> {
		let trimmed = input.trim().strip_prefix('v').unwrap_or(input.trim());
		let without_build = trimmed.split('+').next().unwrap_or(trimmed);
		let (core, pre) = match without_build.split_once('-') {
			Some((c, p)) if !p.is_empty() => (c, Some(p.to_owned())),
			Some(_) => return Err(Error::InvalidArgument),
			None => (without_build, None),
		};
		let mut parts = core.split('.');
		let major = parts
			.next()
			.and_then(|s| s.parse::<u64>().ok())
			.ok_or(Error::InvalidArgument)?;
		let minor = parts
			.next()
			.and_then(|s| s.parse::<u64>().ok())
			.ok_or(Error::InvalidArgument)?;
		let patch = parts
			.next()
			.and_then(|s| s.parse::<u64>().ok())
			.ok_or(Error::InvalidArgument)?;
		if parts.next().is_some() {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			major,
			minor,
			patch,
			pre,
		})
	}

	/// True when this version carries a pre-release tag.
	#[must_use]
	pub const fn is_prerelease(&self) -> bool {
		self.pre.is_some()
	}
}

impl Ord for SemVer {
	fn cmp(&self, other: &Self) -> Ordering {
		(self.major, self.minor, self.patch)
			.cmp(&(other.major, other.minor, other.patch))
			.then_with(|| {
				match (&self.pre, &other.pre) {
					(None, None) => Ordering::Equal,
					(None, Some(_)) => Ordering::Greater,
					(Some(_), None) => Ordering::Less,
					(Some(left), Some(right)) => left.cmp(right),
				}
			})
	}
}

impl PartialOrd for SemVer {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

/// Returns `true` when `candidate` is strictly newer than `current`.
///
/// # Errors
///
/// Returns [`Error::InvalidArgument`] if either string is not valid `SemVer`.
pub fn is_newer_version(current: &str, candidate: &str) -> Result<bool, Error> {
	Ok(SemVer::parse(candidate)? > SemVer::parse(current)?)
}

/// One downloadable platform installer / archive in a [`ReleaseManifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformArtifact {
	/// Download HTTPS URL.
	pub url: String,
	/// Expected lowercase hex SHA-256 digest (64 hex characters).
	pub sha256_hex: String,
	/// Expected size in bytes (must be positive).
	pub size_bytes: u64,
	/// Optional detached cryptographic signature (e.g. Ed25519 base64/hex).
	pub signature: Option<String>,
}

impl PlatformArtifact {
	/// Validates HTTPS URL, 64-char hex SHA-256, and non-zero size.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] if any field is malformed.
	pub fn validate(&self) -> Result<(), Error> {
		if !self.url.starts_with("https://") || self.url.len() <= "https://".len() {
			return Err(Error::InvalidArgument);
		}
		if self.sha256_hex.len() != 64 || !self.sha256_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
			return Err(Error::InvalidArgument);
		}
		if self.size_bytes == 0 {
			return Err(Error::InvalidArgument);
		}
		Ok(())
	}
}

/// Signed desktop release manifest returned by an update server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseManifest {
	/// Release `SemVer` string (e.g. `"2.1.0"`).
	pub version: String,
	/// Target release channel.
	pub channel: UpdateChannel,
	/// Optional human-readable release notes.
	pub notes: Option<String>,
	/// Whether the host should prompt right away for a mandatory update.
	pub mandatory: bool,
	/// Optional staged rollout percentage (`0..=100`).
	pub rollout_percent: Option<u8>,
	/// Per-architecture artifacts.
	pub artifacts: BTreeMap<TargetPlatformId, PlatformArtifact>,
}

impl ReleaseManifest {
	/// Validates version format, rollout percentage, and every artifact.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidArgument`] on invalid fields.
	pub fn validate(&self) -> Result<(), Error> {
		let _ = SemVer::parse(&self.version)?;
		if let Some(percent) = self.rollout_percent
			&& percent > 100
		{
			return Err(Error::InvalidArgument);
		}
		if self.artifacts.is_empty() {
			return Err(Error::InvalidArgument);
		}
		for artifact in self.artifacts.values() {
			artifact.validate()?;
		}
		Ok(())
	}

	/// Looks up the best artifact for `target` (falling back to
	/// [`TargetPlatformId::MacosUniversal`] on macOS targets).
	#[must_use]
	pub fn artifact_for(&self, target: TargetPlatformId) -> Option<&PlatformArtifact> {
		self.artifacts.get(&target).or_else(|| {
			matches!(
				target,
				TargetPlatformId::MacosAarch64 | TargetPlatformId::MacosX86_64
			)
			.then(|| self.artifacts.get(&TargetPlatformId::MacosUniversal))
			.flatten()
		})
	}

	/// Checks whether a machine in deterministic bucket `0..100` is eligible
	/// for this release's staged rollout.
	#[must_use]
	pub fn is_eligible_for_rollout(&self, machine_bucket: u8) -> bool {
		let rollout = self.rollout_percent.unwrap_or(100);
		machine_bucket < rollout
	}
}

/// Host auto-update policy (mirrors Electron `autoUpdater` settings).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct UpdatePolicy {
	/// Release channel to follow.
	pub channel: UpdateChannel,
	/// Optional automatic background check interval in seconds.
	pub auto_check_interval_secs: Option<u64>,
	/// Automatically download once a newer release is discovered.
	pub auto_download: bool,
	/// Apply a staged update automatically when the user quits the app.
	pub install_on_quit: bool,
	/// Allow pre-release versions (`1.2.0-beta.1`) on the subscribed channel.
	pub allow_prerelease: bool,
	/// Refuse artifacts without a verified signature.
	pub require_signature: bool,
}

impl Default for UpdatePolicy {
	fn default() -> Self {
		Self {
			channel: UpdateChannel::Stable,
			auto_check_interval_secs: Some(3600),
			auto_download: false,
			install_on_quit: true,
			allow_prerelease: false,
			require_signature: true,
		}
	}
}

/// Verifies downloaded artifact hashes and signatures before staging.
pub trait IntegrityVerifier: Send {
	/// Verifies the artifact's SHA-256 digest matches `expected_hex`.
	///
	/// # Errors
	///
	/// Returns [`Error::Native`] on digest mismatch.
	fn verify_sha256(&self, payload: &[u8], expected_hex: &str) -> Result<(), Error> {
		let actual = sha256_hex(payload);
		if actual.eq_ignore_ascii_case(expected_hex) {
			Ok(())
		} else {
			Err(Error::Native("SHA-256 digest mismatch".into()))
		}
	}

	/// Verifies the detached signature over `payload`.
	///
	/// # Errors
	///
	/// Returns [`Error::Native`] or [`Error::Unavailable`] when the signature
	/// cannot be verified.
	fn verify_signature(&self, payload: &[u8], signature: &str) -> Result<(), Error>;
}

/// Default SHA-256 verifier. Rejects signature checks unless a host supplies a
/// real verifier (for example Ed25519, Authenticode, or notarization).
#[derive(Debug, Default, Clone, Copy)]
pub struct Sha256OnlyVerifier;

impl IntegrityVerifier for Sha256OnlyVerifier {
	fn verify_signature(&self, _payload: &[u8], _signature: &str) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

/// Host network/disk transport used by [`FeedBackend`].
pub trait FeedTransport: Send {
	/// Fetches the latest manifest for `channel`, or `None` if unavailable.
	fn fetch_manifest(&mut self, channel: &UpdateChannel)
	-> Result<Option<ReleaseManifest>, Error>;

	/// Downloads the raw installer/archive bytes for `artifact`.
	fn download_artifact(&mut self, artifact: &PlatformArtifact) -> Result<Vec<u8>, Error>;

	/// Stages and launches the verified update installer for `version`.
	fn stage_and_install(&mut self, version: &str, bytes: &[u8]) -> Result<(), Error>;
}

/// Production-shaped [`Backend`] driven by a [`FeedTransport`],
/// [`UpdatePolicy`], and [`IntegrityVerifier`].
pub struct FeedBackend<T, V> {
	current_version: String,
	target: TargetPlatformId,
	machine_bucket: u8,
	policy: UpdatePolicy,
	transport: T,
	verifier: V,
	pending_manifest: Option<ReleaseManifest>,
	downloaded: Option<(String, Vec<u8>)>,
}

impl<T, V> FeedBackend<T, V> {
	/// Creates a feed-driven desktop updater backend.
	#[must_use]
	pub fn new(
		current_version: impl Into<String>,
		target: TargetPlatformId,
		policy: UpdatePolicy,
		transport: T,
		verifier: V,
	) -> Self {
		Self {
			current_version: current_version.into(),
			target,
			machine_bucket: 0,
			policy,
			transport,
			verifier,
			pending_manifest: None,
			downloaded: None,
		}
	}

	/// Active update policy.
	#[must_use]
	pub const fn policy(&self) -> &UpdatePolicy {
		&self.policy
	}

	/// Sets the deterministic rollout bucket (`0..100`) for staged rollouts.
	#[must_use]
	pub const fn with_machine_bucket(mut self, bucket: u8) -> Self {
		self.machine_bucket = bucket % 100;
		self
	}
}

impl<T: FeedTransport, V: IntegrityVerifier> Backend for FeedBackend<T, V> {
	fn current_version(&self) -> &str {
		&self.current_version
	}

	fn check(&mut self) -> Result<Status, Error> {
		let Some(manifest) = self.transport.fetch_manifest(&self.policy.channel)? else {
			return Ok(Status::UpToDate);
		};
		manifest.validate()?;

		if manifest.channel != self.policy.channel {
			return Ok(Status::UpToDate);
		}
		if !manifest.is_eligible_for_rollout(self.machine_bucket) {
			return Ok(Status::UpToDate);
		}
		let parsed = SemVer::parse(&manifest.version)?;
		if parsed.is_prerelease() && !self.policy.allow_prerelease {
			return Ok(Status::UpToDate);
		}
		if !is_newer_version(&self.current_version, &manifest.version)? {
			return Ok(Status::UpToDate);
		}
		if manifest.artifact_for(self.target).is_none() {
			return Ok(Status::UpToDate);
		}

		let status = Status::Available {
			version: manifest.version.clone(),
			notes: manifest.notes.clone(),
		};
		self.pending_manifest = Some(manifest);
		Ok(status)
	}

	fn download(&mut self) -> Result<Status, Error> {
		let Some(manifest) = self.pending_manifest.clone() else {
			return Ok(Status::UpToDate);
		};
		let Some(artifact) = manifest.artifact_for(self.target).cloned() else {
			return Err(Error::Unavailable);
		};
		let payload = self.transport.download_artifact(&artifact)?;
		if payload.len() as u64 != artifact.size_bytes {
			return Ok(Status::Failed {
				message: "downloaded artifact size did not match manifest".into(),
			});
		}
		if self
			.verifier
			.verify_sha256(&payload, &artifact.sha256_hex)
			.is_err()
		{
			return Ok(Status::Failed {
				message: "downloaded artifact SHA-256 verification failed".into(),
			});
		}
		if self.policy.require_signature {
			let Some(signature) = artifact.signature.as_deref() else {
				return Ok(Status::Failed {
					message: "update manifest artifact is missing a required signature".into(),
				});
			};
			if self.verifier.verify_signature(&payload, signature).is_err() {
				return Ok(Status::Failed {
					message: "downloaded artifact signature verification failed".into(),
				});
			}
		}

		let version = manifest.version.clone();
		self.downloaded = Some((version.clone(), payload));
		Ok(Status::Ready { version })
	}

	fn install_and_restart(&mut self) -> Result<(), Error> {
		let Some((version, bytes)) = self.downloaded.take() else {
			return Err(Error::Native("no downloaded update is ready".into()));
		};
		self.transport.stage_and_install(&version, &bytes)
	}
}

/// Computes the lowercase hex FIPS 180-4 SHA-256 digest of `data`.
#[must_use]
#[allow(
	clippy::many_single_char_names,
	reason = "standard FIPS 180-4 SHA-256 working variable names"
)]
pub fn sha256_hex(data: &[u8]) -> String {
	#[rustfmt::skip]
	const K: [u32; 64] = [
		0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4,
		0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe,
		0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f,
		0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da, 0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
		0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc,
		0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
		0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070, 0x19a4_c116,
		0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
		0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7,
		0xc671_78f2,
	];

	let mut state: [u32; 8] = [
		0x6a09_e667,
		0xbb67_ae85,
		0x3c6e_f372,
		0xa54f_f53a,
		0x510e_527f,
		0x9b05_688c,
		0x1f83_d9ab,
		0x5be0_cd19,
	];

	let bit_len = (data.len() as u64).wrapping_mul(8);
	let mut padded = Vec::with_capacity(data.len() + 72);
	padded.extend_from_slice(data);
	padded.push(0x80);
	while padded.len() % 64 != 56 {
		padded.push(0);
	}
	padded.extend_from_slice(&bit_len.to_be_bytes());

	for chunk in padded.as_chunks::<64>().0 {
		let mut words = [0u32; 64];
		for (index, &word) in chunk.as_chunks::<4>().0.iter().enumerate() {
			words[index] = u32::from_be_bytes(word);
		}
		for i in 16..64 {
			let s0 = words[i - 15].rotate_right(7)
				^ words[i - 15].rotate_right(18)
				^ (words[i - 15] >> 3);
			let s1 = words[i - 2].rotate_right(17)
				^ words[i - 2].rotate_right(19)
				^ (words[i - 2] >> 10);
			words[i] = words[i - 16]
				.wrapping_add(s0)
				.wrapping_add(words[i - 7])
				.wrapping_add(s1);
		}

		let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
		for i in 0..64 {
			let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
			let ch = (e & f) ^ ((!e) & g);
			let temp1 = h
				.wrapping_add(s1)
				.wrapping_add(ch)
				.wrapping_add(K[i])
				.wrapping_add(words[i]);
			let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
			let maj = (a & b) ^ (a & c) ^ (b & c);
			let temp2 = s0.wrapping_add(maj);

			h = g;
			g = f;
			f = e;
			e = d.wrapping_add(temp1);
			d = c;
			c = b;
			b = a;
			a = temp1.wrapping_add(temp2);
		}

		for (slot, delta) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
			*slot = slot.wrapping_add(delta);
		}
	}

	let mut hex = String::with_capacity(64);
	for word in state {
		let _ = write!(&mut hex, "{word:08x}");
	}
	hex
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Updater;

	struct MockTransport {
		manifest: Option<ReleaseManifest>,
		bytes: Vec<u8>,
		installed_version: Option<String>,
	}

	impl FeedTransport for MockTransport {
		fn fetch_manifest(
			&mut self,
			_channel: &UpdateChannel,
		) -> Result<Option<ReleaseManifest>, Error> {
			Ok(self.manifest.clone())
		}

		fn download_artifact(&mut self, _artifact: &PlatformArtifact) -> Result<Vec<u8>, Error> {
			Ok(self.bytes.clone())
		}

		fn stage_and_install(&mut self, version: &str, _bytes: &[u8]) -> Result<(), Error> {
			self.installed_version = Some(version.to_owned());
			Ok(())
		}
	}

	struct StaticSignatureVerifier(&'static str);

	impl IntegrityVerifier for StaticSignatureVerifier {
		fn verify_signature(&self, _payload: &[u8], signature: &str) -> Result<(), Error> {
			if signature == self.0 {
				Ok(())
			} else {
				Err(Error::Native("invalid signature".into()))
			}
		}
	}

	#[test]
	fn sha256_matches_nist_vectors() {
		assert_eq!(
			sha256_hex(b""),
			"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
		);
		assert_eq!(
			sha256_hex(b"abc"),
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
		);
	}

	#[test]
	fn semver_ordering_and_rollout_gating_work() {
		assert!(is_newer_version("1.0.0", "1.0.1").unwrap());
		assert!(is_newer_version("1.0.0-beta.1", "1.0.0").unwrap());
		assert!(!is_newer_version("2.0.0", "1.9.9").unwrap());
		assert!(is_newer_version("invalid", "1.0.0").is_err());
	}

	#[test]
	fn feed_backend_verifies_hash_and_signature_before_install() {
		let payload = b"installer-bytes-v2".to_vec();
		let digest = sha256_hex(&payload);
		let mut artifacts = BTreeMap::new();
		artifacts.insert(
			TargetPlatformId::MacosUniversal,
			PlatformArtifact {
				url: "https://releases.example.com/app-2.0.0.dmg".into(),
				sha256_hex: digest,
				size_bytes: payload.len() as u64,
				signature: Some("valid-ed25519-sig".into()),
			},
		);
		let manifest = ReleaseManifest {
			version: "2.0.0".into(),
			channel: UpdateChannel::Stable,
			notes: Some("Desktop vibrancy & menus".into()),
			mandatory: false,
			rollout_percent: Some(50),
			artifacts,
		};

		// Machine bucket 75 is outside 50% rollout.
		let gated = FeedBackend::new(
			"1.0.0",
			TargetPlatformId::MacosAarch64,
			UpdatePolicy::default(),
			MockTransport {
				manifest: Some(manifest.clone()),
				bytes: payload.clone(),
				installed_version: None,
			},
			StaticSignatureVerifier("valid-ed25519-sig"),
		)
		.with_machine_bucket(75);
		let mut updater_gated = Updater::new(gated);
		assert_eq!(updater_gated.check().unwrap(), &Status::UpToDate);

		// Machine bucket 10 gets the update, downloads, verifies, and installs.
		let backend = FeedBackend::new(
			"1.0.0",
			TargetPlatformId::MacosAarch64,
			UpdatePolicy::default(),
			MockTransport {
				manifest: Some(manifest.clone()),
				bytes: payload,
				installed_version: None,
			},
			StaticSignatureVerifier("valid-ed25519-sig"),
		)
		.with_machine_bucket(10);
		let mut updater = Updater::new(backend);
		assert_eq!(
			updater.check().unwrap(),
			&Status::Available {
				version: "2.0.0".into(),
				notes: Some("Desktop vibrancy & menus".into()),
			}
		);
		assert_eq!(
			updater.download().unwrap(),
			&Status::Ready {
				version: "2.0.0".into(),
			}
		);
		updater.install_and_restart().unwrap();

		// Tampered bytes fail SHA-256 verification.
		let tampered = FeedBackend::new(
			"1.0.0",
			TargetPlatformId::MacosAarch64,
			UpdatePolicy::default(),
			MockTransport {
				manifest: Some(manifest),
				bytes: b"tampered-bytes-v2!".to_vec(),
				installed_version: None,
			},
			StaticSignatureVerifier("valid-ed25519-sig"),
		);
		let mut updater_bad = Updater::new(tampered);
		assert!(matches!(
			updater_bad.check().unwrap(),
			Status::Available { .. }
		));
		assert!(matches!(
			updater_bad.download().unwrap(),
			Status::Failed { .. }
		));
	}
}
