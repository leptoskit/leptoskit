//! <!-- {=watchSummary|trim|linePrefix:"//! ":true} -->
//!
//! Watch apps are not phone apps with a smaller viewport. This crate models the
//! surfaces Wear OS and watchOS actually give you — interactive pages, tiles,
//! glances, and complications — and lets each one carry a leptoskit `Element`
//! tree under a hard layout budget. Phone file routes do not map onto these
//! surfaces.
//!
//! <!-- {/watchSummary} -->
//!
//! # Platforms
//!
//! | Platform | Crate | Status |
//! | --- | --- | --- |
//! | Android Wear OS | [`leptoskit_wearos`](https://docs.rs/leptoskit_wearos) | Experimental stubs |
//! | Apple watchOS | [`leptoskit_watchos`](https://docs.rs/leptoskit_watchos) | Experimental stubs |
//!
//! Both backends implement [`WatchHost`]. Until a host is wired to the device
//! toolkit, presentations are recorded in memory so guests and tests still
//! compile on every machine.

#![forbid(unsafe_code)]

mod ambient;
pub mod aod;
mod app;
mod budget;
pub mod companion;
pub mod crown;
mod error;
mod host;
pub mod interaction;
mod kind;
mod navigation;
mod page;
mod viewport;
mod wearable;
pub mod workout;

pub use ambient::ComplicationFamily;
pub use ambient::RefreshPolicy;
pub use ambient::WatchAmbient;
pub use ambient::WatchComplication;
pub use ambient::WatchGlance;
pub use ambient::WatchTile;
pub use aod::AmbientModeState;
pub use aod::AmbientPolicy;
pub use aod::RelevanceHint;
pub use app::WatchApp;
pub use budget::WatchBudget;
pub use budget::measure;
pub use companion::CompanionLink;
pub use companion::CompanionMessage;
pub use companion::CompanionState;
pub use companion::DeliveryMode as CompanionDeliveryMode;
pub use companion::MemoryCompanionLink;
pub use crown::CrownConfig;
pub use crown::CrownEvent;
pub use crown::CrownSensitivity;
pub use crown::RotaryState;
pub use error::WatchError;
pub use host::RecordingHost;
pub use host::WatchHost;
pub use host::WatchPresentation;
pub use interaction::WatchCapabilities;
pub use interaction::WatchGesture;
pub use interaction::WatchHaptic;
pub use kind::WatchKind;
pub use navigation::WatchNavigation;
pub use page::WatchPage;
pub use viewport::WatchShape;
pub use viewport::WatchSizeClass;
pub use viewport::WatchViewport;
pub use wearable::DeliveryMode;
pub use wearable::SamplingPolicy;
pub use wearable::WearableApp;
pub use wearable::WearableStatus;
pub use wearable::WearableStream;
pub use wearable::WearableStreams;
pub use wearable::WearableSurfaceMode;
pub use workout::WorkoutActivity;
pub use workout::WorkoutLocation;
pub use workout::WorkoutMetrics;
pub use workout::WorkoutSession;
pub use workout::WorkoutState;
