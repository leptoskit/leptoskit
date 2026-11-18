//! How big a process is, how many replicas run, whether they sleep, and the
//! database and region an environment uses.
//!
//! | Size | CPU request / limit (millicores) | Memory (MiB) |
//! | ---- | -------------------------------- | ------------ |
//! | xs   | 100 / 500                        | 128          |
//! | s    | 250 / 1000                       | 256          |
//! | m    | 500 / 2000                       | 512          |
//! | l    | 1000 / 2000                      | 1024         |
//! | xl   | 2000 / 4000                      | 2048         |
//!
//! [`Scale::new`] holds `min <= max` and `1 <= max <= 20`; [`check_sleep`]
//! holds that sleeping when idle is `min = 0`. A [`DatabaseSize`] is
//! `shared` or a dedicated instance with 1, 2, or 4 GiB.
//!
//! The types are defined once, in [`leptoskit_config::cloud`], because the
//! `[cloud]` table parses them; this module re-exports them so the cloud's
//! own code has one place to name them from.

pub use leptoskit_config::cloud::Backups;
pub use leptoskit_config::cloud::DatabaseInstance;
pub use leptoskit_config::cloud::DatabaseSize;
pub use leptoskit_config::cloud::MAX_REPLICAS;
pub use leptoskit_config::cloud::Region;
pub use leptoskit_config::cloud::RegionError;
pub use leptoskit_config::cloud::Scale;
pub use leptoskit_config::cloud::ScaleError;
pub use leptoskit_config::cloud::Size;
pub use leptoskit_config::cloud::Sleep;
pub use leptoskit_config::cloud::SleepError;
pub use leptoskit_config::cloud::UnknownChoice;
pub use leptoskit_config::cloud::check_sleep;
