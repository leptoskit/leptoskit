//! Deterministic ownership for the deployment graph.
//!
//! Routes and artifact trees use the same unit identities. A frontend can
//! share a build recipe with another frontend without sharing its public
//! paths or the files published in its release. These checks do not access
//! provider APIs, run build commands, or attest to running infrastructure.

pub mod artifacts;
pub mod plan;
pub mod routes;

/// An independently released member of a project.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unit {
	/// A static or server-rendered frontend.
	Frontend(String),
	/// An application service with its own process and release.
	Service(String),
}

impl Unit {
	/// The name from the project's declaration, without its kind prefix.
	#[must_use]
	pub fn name(&self) -> &str {
		match self {
			Self::Frontend(name) | Self::Service(name) => name,
		}
	}

	fn validate(&self) -> Result<(), String> {
		identifier(self.name()).then_some(()).ok_or_else(|| {
			format!(
				"invalid graph unit name {:?}; use 1–63 letters, digits, hyphens or underscores",
				self.name()
			)
		})
	}
}

impl std::fmt::Display for Unit {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let kind = match self {
			Self::Frontend(_) => "frontends",
			Self::Service(_) => "services",
		};

		write!(f, "{kind}.{}", self.name())
	}
}

pub(crate) fn identifier(value: &str) -> bool {
	value.len() <= 63 && artifact_identifier(value)
}

// Derived service/toolchain artifact names include a prefix as well as the
// complete unit name. They must remain collision-free at the unit-name bound.
fn artifact_identifier(value: &str) -> bool {
	(1..=128).contains(&value.len())
		&& value.as_bytes()[0].is_ascii_alphanumeric()
		&& value
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

/// Reject ambiguous spellings instead of guessing how a filesystem, proxy
/// or web server would normalize them. Filesystem containment and symlink
/// checks must also run when the actual build output is consumed.
fn components(path: &str) -> bool {
	!path.is_empty()
		&& path.split('/').all(|part| {
			!part.is_empty()
				&& part != "."
				&& part != ".."
				&& !part.bytes().any(|b| b.is_ascii_control() || b == b'\\')
		})
}
