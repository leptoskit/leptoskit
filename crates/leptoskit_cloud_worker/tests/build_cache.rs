//! The build cache that every organisation's builds share on a build worker:
//! what one organisation's build leaves on it, and what the next
//! organisation's build finds there.
//!
//! Everything run is the worker's own. `CommandBuilder` builds through this
//! test binary started a second time, in place of the worker's binary: that
//! second process runs `cli::execute` on `build-app`, with the arguments clap
//! reads from the variables `CommandBuilder` hands it, as the worker's `main`
//! does. `build-app` then plans, exports and rotates the `BuildKit` cache
//! itself.
//!
//! `BuildKit` alone is stood in for, by a `buildctl` script that writes into
//! the export directory `build-app` names one incompressible layer of the
//! size the tenant's Dockerfile asks for. The script stands in for the
//! volume's size too: the volume is a directory here, which has none of its
//! own, so the script refuses, as a full one does, a layer that would take
//! what the directory holds past the capacity of the volume it stands in
//! for.

#![cfg(unix)]

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;

use clap::Parser as _;
use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::deployments::AppFacts;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::BuildServerInput;
use leptoskit_cloud::workers::DeploymentRef;
use leptoskit_cloud::workers::Download;
use leptoskit_cloud_worker::BuildJob;
use leptoskit_cloud_worker::Builder as _;
use leptoskit_cloud_worker::CommandBuilder;
use leptoskit_cloud_worker::LogSink;
use leptoskit_cloud_worker::Stop;

/// The capacity, in KiB, of the volume the stand-in writes to: that of a
/// small one, which holds `ana`'s export or `bob`'s, and not both.
const CAPACITY_KIB: u64 = 64 * 1024;

/// The most of the cache a project may keep in the test of an oversized
/// export, in MiB: more than `bob`'s export and less than `ana`'s.
const KEEP_MIB: u64 = 32;

/// The most of the cache a project may keep in the test of stale caches, in
/// MiB: `ana`'s export is within it, so only its going stale takes it off
/// the volume.
const KEEPS_LARGE_MIB: u64 = 64;

/// What `ana`'s Dockerfile asks for, in MiB.
const LARGE: u32 = 56;

/// What `bob`'s asks for.
const SMALL: u32 = 12;

/// Stands in for `buildctl build`: honours `--export-cache type=local,dest=…`
/// and `--metadata-file`, and writes into that directory one layer as large
/// as the Dockerfile's `RUN head -c <n>M …` step, unless the volume
/// `LK_STAND_IN_VOLUME`, of `LK_STAND_IN_CAPACITY_KIB` KiB, has no room for
/// it.
const BUILDCTL: &str = r#"#!/bin/sh
dir=.
file=Dockerfile
meta=
spec=
while [ $# -gt 0 ]; do
	case "$1" in
	--metadata-file) meta=$2; shift 2 ;;
	--export-cache) spec=$2; shift 2 ;;
	--local)
		case "$2" in dockerfile=*) dir=${2#dockerfile=} ;; esac
		shift 2 ;;
	--opt)
		case "$2" in filename=*) file=${2#filename=} ;; esac
		shift 2 ;;
	*) shift ;;
	esac
done
mib=
while IFS= read -r line; do
	case "$line" in
	"RUN head -c "*)
		mib=${line#"RUN head -c "}
		mib=${mib%%M*}
		;;
	esac
done < "$dir/$file"
[ -n "$mib" ] || mib=1
dest=${spec#*dest=}
dest=${dest%%,*}
echo "stand-in buildctl: exporting $mib MiB of layers to $dest"
used=$(du -sk "$LK_STAND_IN_VOLUME" | cut -f1)
if [ "$((used + mib * 1024))" -gt "$LK_STAND_IN_CAPACITY_KIB" ]; then
	echo "stand-in buildctl: no space left on device: $used KiB of the volume's $LK_STAND_IN_CAPACITY_KIB KiB are taken, and $mib MiB more would not fit"
	exit 1
fi
mkdir -p "$dest"
if dd if=/dev/urandom of="$dest/layer-1.bin" bs=1048576 count="$mib" 2>&1; then
	printf '{"schemaVersion":2}\n' >"$dest/index.json"
	printf '{"containerimage.digest":"sha256:%s"}' 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef >"$meta"
	exit 0
fi
echo "stand-in buildctl: the export to $dest did not finish"
exit 1
"#;

/// The `build-app` subcommand, as the worker's binary runs it (`main.rs`):
/// this test binary, started by `CommandBuilder` in its place. An ordinary
/// run never hands it a build, and skips it.
#[test]
#[ignore = "started by CommandBuilder, as the worker's build command"]
fn build_app_process() {
	if std::env::var_os("LEPTOSKIT_BUILD_INPUT").is_none() {
		return;
	}

	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("a runtime");

	runtime.block_on(async {
		let (_sender, shutdown) = tokio::sync::watch::channel(false);
		let command = leptoskit_cloud_worker::cli::Command::parse_from([
			"leptoskit-cloud-worker",
			"build-app",
		]);

		if let Err(error) = leptoskit_cloud_worker::cli::execute(command, shutdown).await {
			eprintln!("leptoskit-cloud-worker: {error}");
			std::process::exit(1);
		}
	});
}

/// A volume that every organisation's builds share, and the stand-in for
/// `buildctl` that writes to it and refuses what would overfill it.
struct Shared {
	/// Stands in for the `build-worker-cache` volume.
	volume: tempfile::TempDir,
	/// Where the stand-in for `buildctl` is put, for `PATH` to find.
	bin: tempfile::TempDir,
}

impl Shared {
	fn new() -> Self {
		use std::os::unix::fs::PermissionsExt as _;

		let bin = tempfile::tempdir().expect("a bin directory");
		let program = bin.path().join("buildctl");
		std::fs::write(&program, BUILDCTL).expect("the stand-in for buildctl");
		std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
			.expect("an executable stand-in");

		Self {
			volume: tempfile::tempdir().expect("a volume"),
			bin,
		}
	}

	/// The volume.
	fn path(&self) -> &Path {
		self.volume.path()
	}

	/// The worker's builder, running the stand-in for `buildctl`: first on
	/// the build's `PATH`, and told which directory stands in for the volume
	/// and how much room the volume has. The wrapper the worker runs the
	/// build under hands it these, so nothing here changes this process's
	/// own environment.
	fn builder(&self) -> CommandBuilder {
		let mut path = OsString::from(self.bin.path());
		path.push(":");
		path.push(std::env::var_os("PATH").unwrap_or_default());
		let mut wrapper = vec![
			format!("PATH={}", path.to_string_lossy()),
			"LEPTOSKIT_CLOUD_BUILDER_BACKEND=buildctl".to_owned(),
			format!("LK_STAND_IN_VOLUME={}", self.volume.path().display()),
			format!("LK_STAND_IN_CAPACITY_KIB={CAPACITY_KIB}"),
		];
		wrapper.insert(0, "/usr/bin/env".to_owned());

		CommandBuilder::new(
			std::env::current_exe().expect("this test binary"),
			vec![
				"--exact".to_owned(),
				"build_app_process".to_owned(),
				"--ignored".to_owned(),
				"--nocapture".to_owned(),
			],
		)
		.sandbox(wrapper)
		.cache_dir(self.volume.path())
	}
}

fn input(organization: &str, project: &str) -> BuildServerInput {
	BuildServerInput {
		deployment: DeploymentRef {
			organization: organization.to_owned(),
			project: project.to_owned(),
			environment: "production".to_owned(),
			number: 3,
		},
		archive: Download {
			url: "https://store.example.test/source".to_owned(),
			expires_at: "2026-11-19T00:00:00Z".parse().expect("a time"),
		},
		digest: Digest::of(b"source"),
		size: 6,
		toolchain: "stable".to_owned(),
		targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
		image_repository: format!("registry.example.test/leptoskit/{organization}/{project}"),
		app: AppFacts::default(),
	}
}

/// One organisation's build of one project through the worker's own
/// builder, on the shared volume: its outcome and its log. `bounds` sets
/// what the worker may do to the cache.
async fn build(
	shared: &Shared,
	organization: &str,
	project: &str,
	layer: u32,
	bounds: impl FnOnce(CommandBuilder) -> CommandBuilder,
) -> (BuildOutcome, Vec<String>) {
	let directory = tempfile::tempdir().expect("a directory");
	let source = directory.path().join("source");
	let scratch = directory.path().join("scratch");
	std::fs::create_dir_all(&source).expect("a source");
	std::fs::create_dir_all(&scratch).expect("a scratch");
	std::fs::write(
		source.join("Dockerfile"),
		format!("FROM scratch\nRUN head -c {layer}M /dev/urandom > /big\n"),
	)
	.expect("the tenant's Dockerfile");
	let input = input(organization, project);
	let (log, mut lines) = LogSink::collecting();
	let outcome = bounds(shared.builder())
		.build(BuildJob {
			input: &input,
			source: &source,
			scratch: &scratch,
			log,
			stop: Stop::never(),
		})
		.await;
	let mut written = Vec::new();

	while let Ok(line) = lines.try_recv() {
		written.push(line);
	}

	(outcome, written)
}

/// How many bytes the files under `path` hold. Links are not followed.
fn bytes_under(path: &Path) -> u64 {
	let Ok(entries) = std::fs::read_dir(path) else {
		return 0;
	};

	entries
		.flatten()
		.map(|entry| {
			let path = entry.path();

			match std::fs::symlink_metadata(&path) {
				Ok(metadata) if metadata.is_dir() => bytes_under(&path),
				Ok(metadata) => metadata.len(),
				Err(_) => 0,
			}
		})
		.sum()
}

/// Notes that the cache at `directory` was last used `ago`.
fn last_used(directory: &Path, ago: Duration) {
	let used = SystemTime::now()
		.checked_sub(ago)
		.expect("a time in the past");

	std::fs::File::open(directory)
		.expect("the cache")
		.set_modified(used)
		.expect("last used");
}

/// A project may keep `KEEP_MIB` of cache, and nothing is removed for the
/// volume being full: the directory here is on this machine's own volume,
/// how full that is isn't the test's to say, and the next bound has its own
/// tests.
fn within_the_maximum(builder: CommandBuilder) -> CommandBuilder {
	builder.cache_max_mib(KEEP_MIB).cache_max_used(100)
}

/// One organisation's build leaves an export too large for any project to
/// keep. It doesn't stay on the volume, so the next organisation's build,
/// which fits an empty volume, still has room: one organisation's build
/// never decides whether another's can run.
#[tokio::test]
async fn an_oversized_export_is_not_left_to_decide_whether_another_organisation_builds() {
	let shared = Shared::new();

	// Bob's build fits an empty volume.
	let (outcome, log) = build(&shared, "bob", "site", SMALL, within_the_maximum).await;
	assert!(
		matches!(outcome, BuildOutcome::Succeeded(_)),
		"control: {outcome:?} {log:?}"
	);
	std::fs::remove_dir_all(shared.path().join("tenants")).expect("an empty volume again");

	// Ana's build, with a layer larger than any project may keep. The build
	// itself succeeds as ever.
	let (outcome, log) = build(&shared, "ana", "notes", LARGE, within_the_maximum).await;
	let ana = shared.path().join("tenants/ana");

	assert!(
		matches!(outcome, BuildOutcome::Succeeded(_)),
		"ana: {outcome:?} {log:?}"
	);
	assert!(
		bytes_under(&ana) <= KEEP_MIB * 1024 * 1024,
		"{} bytes of ana's export stay on the volume",
		bytes_under(&ana)
	);

	// Bob's build again, on the volume as ana's build left it.
	let (outcome, log) = build(&shared, "bob", "site", SMALL, within_the_maximum).await;

	assert!(
		matches!(outcome, BuildOutcome::Succeeded(_)),
		"bob's build on the volume ana's export fills: {outcome:?} {log:?}"
	);
}

/// Another project's cache that no build has used for longer than the idle
/// age is removed before a build, so the build has room. The building
/// project's own cache, however long it has gone unused, is not.
#[tokio::test]
async fn a_stale_cache_of_another_project_makes_room_and_the_building_projects_own_stays() {
	let shared = Shared::new();
	let day = Duration::from_hours(24);
	let keeps_it =
		|builder: CommandBuilder| builder.cache_max_mib(KEEPS_LARGE_MIB).cache_max_used(100);

	// Ana's build: its export is kept, as any within the maximum is. Her
	// project then goes a month without a build.
	let (outcome, log) = build(&shared, "ana", "notes", LARGE, keeps_it).await;

	assert!(
		matches!(outcome, BuildOutcome::Succeeded(_)),
		"ana: {outcome:?} {log:?}"
	);
	let ana = shared.path().join("tenants/ana/notes");
	assert!(
		ana.join("buildkit/index.json").is_file(),
		"ana's export, within the maximum, wasn't kept"
	);
	last_used(&ana, 30 * day);

	// Bob's project has a cache of its own from before, just as old.
	let bob = shared.path().join("tenants/bob/site");
	std::fs::create_dir_all(bob.join("cargo")).expect("bob's cache");
	std::fs::write(bob.join("cargo/kept"), b"from an earlier build").expect("a file");
	last_used(&bob, 30 * day);

	// A build of bob's, told that a cache unused for a day is stale.
	let (outcome, log) = build(&shared, "bob", "site", SMALL, |builder| {
		keeps_it(builder).cache_idle(day)
	})
	.await;

	assert!(
		matches!(outcome, BuildOutcome::Succeeded(_)),
		"bob's build on the volume ana's export fills: {outcome:?} {log:?}"
	);
	assert!(!ana.exists(), "ana's stale cache stays on the volume");
	assert!(
		bob.join("cargo/kept").is_file(),
		"bob's own cache was removed"
	);
}
