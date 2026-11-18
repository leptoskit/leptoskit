//! The runtime side of `LeptosKit` Cloud: the desired state of one
//! environment as plain data, the boundary a place that runs environments
//! sits behind, and the Kubernetes manifests for one release.
//!
//! Nothing here talks to a cluster. [`ReleaseSpec`] is the whole release —
//! processes, routes, policy — that the control plane builds and a
//! [`Runtime`] applies; [`manifests`] turns it into the objects a
//! Kubernetes cluster needs, and the `testing` feature's [`Fake`] records
//! what an adapter was asked to do. The cluster adapter over `kube` and the
//! local process adapter are later slices.
//!
//! # Features
//!
//! | Feature | What it turns on |
//! | --- | --- |
//! | none | [`spec`] and [`runtime`]: pure data, safe in a browser build |
//! | `kubernetes` | [`kubernetes`]: the manifest generator and its types |
//! | `testing` | [`testing`]: the recording fake |
//!
//! The crate depends on nothing else in the cloud: the dependency direction
//! is `leptoskit_cloud_runtime` ← `leptoskit_cloud` ← everything above it.

pub mod runtime;
pub mod spec;

#[cfg(feature = "kubernetes")]
pub mod kubernetes;
#[cfg(feature = "testing")]
pub mod testing;

#[cfg(feature = "kubernetes")]
pub use kubernetes::ClusterProfile;
#[cfg(feature = "kubernetes")]
pub use kubernetes::ManifestError;
#[cfg(feature = "kubernetes")]
pub use kubernetes::Object;
#[cfg(feature = "kubernetes")]
pub use kubernetes::SecretValues;
#[cfg(feature = "kubernetes")]
pub use kubernetes::manifests;
pub use runtime::Applied;
pub use runtime::LogLine;
pub use runtime::LogQuery;
pub use runtime::Observed;
pub use runtime::ObservedProcess;
pub use runtime::ProcessState;
pub use runtime::Runtime;
pub use runtime::RuntimeError;
pub use runtime::RuntimeFuture;
pub use spec::DatabaseAccess;
pub use spec::Egress;
pub use spec::EnvironmentRef;
pub use spec::Image;
pub use spec::NamedRef;
pub use spec::Policy;
pub use spec::Probes;
pub use spec::Process;
pub use spec::ProcessKind;
pub use spec::Quota;
pub use spec::ReleaseSpec;
pub use spec::Resources;
pub use spec::Route;
pub use spec::Scale;
pub use spec::SpecError;
pub use spec::Tls;
pub use spec::Volume;
#[cfg(feature = "testing")]
pub use testing::Call;
#[cfg(feature = "testing")]
pub use testing::Fake;
#[cfg(feature = "testing")]
pub use testing::Script;
