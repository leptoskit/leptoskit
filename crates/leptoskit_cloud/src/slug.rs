//! Slugs: the short names organisations, projects, environments, and
//! templates go by in addresses and commands.
//!
//! A slug matches `[a-z][a-z0-9-]{1,38}[a-z0-9]`, has no `--`, and is none of
//! [`RESERVED_SLUGS`]. [`Slug::suggest`] turns a display name, such as
//! `"My Notes App"`, into one. The type is defined once, in
//! [`leptoskit_config::cloud`], because `[cloud] project` is a slug too.

pub use leptoskit_config::cloud::RESERVED_SLUGS;
pub use leptoskit_config::cloud::SLUG_MAX_LEN;
pub use leptoskit_config::cloud::SLUG_MIN_LEN;
pub use leptoskit_config::cloud::SLUG_PATTERN;
pub use leptoskit_config::cloud::Slug;
pub use leptoskit_config::cloud::SlugError;
