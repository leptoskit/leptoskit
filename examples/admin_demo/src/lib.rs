//! The admin demo's services as a library: the analytics service with the
//! month of traffic that seeds it, and the deploy executor that rehearses
//! operations without a cloud provider.
//!
//! The demo binary, `src/main.rs`, wires these into an admin and its main
//! site. They live here so a demo that serves them another way — such as a
//! public one that reseeds on a schedule — reuses the same services, the
//! same typed events, and the same seeding path.

pub mod analytics;
pub mod executor;
