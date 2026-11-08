//! The Meta Quest build: the Android host, packaged for Horizon OS.
//!
//! Quest runs Android apps, so `--platform quest` builds the app's Android
//! project with the manifest Horizon OS needs: OpenXR's permissions and
//! runtime queries, head tracking, hand tracking and passthrough as
//! configured, the headsets it supports, a full-screen landscape activity
//! with no phone chrome, and, for immersive apps, the categories that launch
//! the app in the headset rather than in a panel.

use std::fmt::Write as _;
use std::ops::Range;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::QuestConfig;
use leptoskit_config::Requirement;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";
/// The full-screen theme with no action bar or status bar.
const THEME: &str = "@android:style/Theme.Black.NoTitleBar.Fullscreen";
/// Every configuration change a headset can make, handled by the activity so
/// the session survives it.
const CONFIG_CHANGES: &str =
	"density|keyboard|keyboardHidden|navigation|orientation|screenLayout|screenSize|uiMode";
/// The OpenXR loader's broker providers and services, which Android 11's
/// package visibility hides unless the app declares them.
const OPENXR_QUERIES: &str = "<provider {p}:authorities=\"org.khronos.openxr.runtime_broker;org.\
                              khronos.openxr.system_runtime_broker\"/><intent><action \
                              {p}:name=\"org.khronos.openxr.OpenXRRuntimeService\"/></\
                              intent><intent><action \
                              {p}:name=\"org.khronos.openxr.OpenXRApiLayerService\"/></intent>";

type Edit = (Range<usize>, String);

/// The Quest manifest for an Android manifest that leptoskit already merged
/// plugin declarations into.
///
/// # Errors
///
/// When the manifest isn't valid XML or has no launcher activity.
pub fn manifest(source: &str, quest: &QuestConfig) -> Result<String> {
	let document = roxmltree::Document::parse(source).context("invalid AndroidManifest.xml")?;
	let root = document.root_element();
	let prefix = root
		.lookup_prefix(ANDROID)
		.context("Android manifest must declare the Android attribute namespace")?;
	let application = root
		.children()
		.find(|node| node.has_tag_name("application"))
		.context("Android manifest has no application element")?;
	let activity = launcher(application)
		.context("the Quest build needs a launcher activity in the Android manifest")?;
	let mut edits: Vec<Edit> = Vec::new();

	let at = application.range().start;
	// Edits at the same place apply in reverse, so the declarations, pushed
	// second, land before the queries.
	edits.push(queries(source, root, prefix)?);
	edits.push((at..at, declarations(root, prefix, quest)?));
	edits.push(application_metadata(application, prefix, quest)?);
	edits.extend(activity_attributes(source, activity, prefix));

	if quest.immersive {
		edits.push(immersive_categories(activity, prefix)?);
	}

	edits.sort_by_key(|edit| std::cmp::Reverse((edit.0.start, edit.0.end)));
	let mut output = source.to_owned();

	for (range, replacement) in edits {
		output.replace_range(range, &replacement);
	}

	roxmltree::Document::parse(&output).context("generated Quest manifest is invalid")?;
	Ok(output)
}

/// The activity whose intent filter has the launcher category.
fn launcher<'a, 'input>(
	application: roxmltree::Node<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
	application
		.children()
		.filter(|node| node.has_tag_name("activity"))
		.find(|activity| launcher_category(*activity).is_some())
}

fn launcher_category<'a, 'input>(
	activity: roxmltree::Node<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
	activity
		.children()
		.filter(|node| node.has_tag_name("intent-filter"))
		.flat_map(|filter| filter.children())
		.find(|node| {
			node.has_tag_name("category")
				&& node.attribute((ANDROID, "name")) == Some("android.intent.category.LAUNCHER")
		})
}

fn named<'a>(root: roxmltree::Node<'a, '_>, tag: &str) -> Vec<&'a str> {
	root.children()
		.filter(|node| node.has_tag_name(tag))
		.filter_map(|node| node.attribute((ANDROID, "name")))
		.collect()
}

/// The features and permissions Horizon OS reads, leaving out any the app
/// already declares.
fn declarations(
	root: roxmltree::Node<'_, '_>,
	prefix: &str,
	quest: &QuestConfig,
) -> Result<String> {
	let features = named(root, "uses-feature");
	let permissions = named(root, "uses-permission");
	let mut output = String::new();
	let mut feature = |name: &str, required: bool, version: Option<&str>| -> Result<()> {
		if !features.contains(&name) {
			let version = version.map_or(String::new(), |v| format!(" {prefix}:version=\"{v}\""));
			writeln!(
				output,
				"<uses-feature {prefix}:name=\"{name}\" \
				 {prefix}:required=\"{required}\"{version}/>"
			)?;
		}
		Ok(())
	};

	feature("android.hardware.vr.headtracking", true, Some("1"))?;

	if quest.hand_tracking != Requirement::Off {
		feature(
			"oculus.software.handtracking",
			quest.hand_tracking == Requirement::Required,
			None,
		)?;
	}

	if quest.passthrough {
		feature("com.oculus.feature.PASSTHROUGH", false, None)?;
	}

	let mut wanted = vec![
		"org.khronos.openxr.permission.OPENXR",
		"org.khronos.openxr.permission.OPENXR_SYSTEM",
	];

	if quest.hand_tracking != Requirement::Off {
		wanted.push("com.oculus.permission.HAND_TRACKING");
	}

	for permission in wanted {
		if !permissions.contains(&permission) {
			writeln!(output, "<uses-permission {prefix}:name=\"{permission}\"/>")?;
		}
	}

	Ok(output)
}

/// OpenXR's queries, in the app's `<queries>` element or a new one.
fn queries(source: &str, root: roxmltree::Node<'_, '_>, prefix: &str) -> Result<Edit> {
	let entries = OPENXR_QUERIES.replace("{p}", prefix);
	let Some(queries) = root.children().find(|node| node.has_tag_name("queries")) else {
		let at = root
			.children()
			.find(|node| node.has_tag_name("application"))
			.context("Android manifest has no application element")?
			.range()
			.start;
		return Ok((at..at, format!("<queries>{entries}</queries>\n")));
	};
	let end = queries.range().end;
	let close = source[..end]
		.rfind("</queries>")
		.filter(|close| *close > queries.range().start)
		.context("the Android manifest's queries element must not be self-closing")?;

	Ok((close..close, entries))
}

/// The supported headsets and the hand-tracking version, first in the
/// application element.
fn application_metadata(
	application: roxmltree::Node<'_, '_>,
	prefix: &str,
	quest: &QuestConfig,
) -> Result<Edit> {
	let at = application
		.first_child()
		.context("the Android manifest's application element must not be empty")?
		.range()
		.start;
	let devices: Vec<String> = quest
		.supported_devices
		.iter()
		.map(ToString::to_string)
		.collect();
	let mut metadata = format!(
		"\n<meta-data {prefix}:name=\"com.oculus.supportedDevices\" {prefix}:value=\"{}\"/>",
		devices.join("|")
	);

	if quest.hand_tracking != Requirement::Off {
		write!(
			metadata,
			"\n<meta-data {prefix}:name=\"com.oculus.handtracking.version\" \
			 {prefix}:value=\"V2.0\"/>"
		)?;
	}

	Ok((at..at, metadata))
}

/// Full screen, landscape, no picture-in-picture, and every configuration
/// change handled in place.
fn activity_attributes(source: &str, activity: roxmltree::Node<'_, '_>, prefix: &str) -> Vec<Edit> {
	let wanted = [
		("theme", Some(THEME)),
		("screenOrientation", Some("landscape")),
		("configChanges", Some(CONFIG_CHANGES)),
		("resizeableActivity", Some("false")),
		("launchMode", Some("singleTask")),
		("excludeFromRecents", Some("false")),
		("supportsPictureInPicture", None),
	];
	let insert = activity.range().start + "<activity".len();
	let mut edits = Vec::new();

	for (name, value) in wanted {
		let existing = activity
			.attributes()
			.find(|attribute| attribute.namespace() == Some(ANDROID) && attribute.name() == name);
		let replacement = value.map_or(String::new(), |value| {
			format!("{prefix}:{name}=\"{value}\"")
		});

		match existing {
			Some(attribute) => {
				let range = attribute.range();
				// Removing an attribute takes the space before it too.
				let start = if value.is_none() && source[..range.start].ends_with(' ') {
					range.start - 1
				} else {
					range.start
				};
				edits.push((start..range.end, replacement));
			}
			None if value.is_some() => edits.push((insert..insert, format!(" {replacement}"))),
			None => {}
		}
	}

	edits
}

/// The categories that launch the activity in the headset.
fn immersive_categories(activity: roxmltree::Node<'_, '_>, prefix: &str) -> Result<Edit> {
	let at = launcher_category(activity)
		.context("the launcher activity lost its category")?
		.range()
		.end;

	Ok((
		at..at,
		format!(
			"\n<category {prefix}:name=\"com.oculus.intent.category.VR\"/>\n<category \
			 {prefix}:name=\"org.khronos.openxr.intent.category.IMMERSIVE_HMD\"/>"
		),
	))
}

#[cfg(test)]
mod tests {
	use leptoskit_config::QuestDevice;

	use super::*;

	/// The manifest `leptoskit new` writes, with one plugin permission merged
	/// in.
	fn android() -> String {
		include_str!("templates/AndroidManifest.xml")
			.replace("__NAME__", "Reapers")
			.replace("__PACKAGE__", "dev.reapers.app")
			.replace(
				"<application",
				"<uses-permission android:name=\"android.permission.CAMERA\"/>\n<application",
			)
	}

	#[test]
	fn an_immersive_quest_manifest() {
		let manifest = manifest(&android(), &QuestConfig::default()).unwrap();
		insta::assert_snapshot!(manifest);
	}

	#[test]
	fn a_panel_app_without_hands_or_passthrough() {
		let quest = QuestConfig {
			immersive: false,
			supported_devices: vec![QuestDevice::Quest3],
			hand_tracking: Requirement::Off,
			passthrough: false,
		};
		let manifest = manifest(&android(), &quest).unwrap();
		assert!(!manifest.contains("com.oculus.intent.category.VR"));
		assert!(!manifest.contains("HAND_TRACKING"));
		assert!(!manifest.contains("PASSTHROUGH"));
		assert!(manifest.contains("android:value=\"quest3\""));
		assert!(manifest.contains("android.hardware.vr.headtracking"));
		assert!(!manifest.contains("supportsPictureInPicture"));
	}

	#[test]
	fn required_hand_tracking_is_required() {
		let quest = QuestConfig {
			hand_tracking: Requirement::Required,
			..QuestConfig::default()
		};
		let manifest = manifest(&android(), &quest).unwrap();
		assert!(manifest.contains(
			"<uses-feature android:name=\"oculus.software.handtracking\" \
			 android:required=\"true\"/>"
		));
	}

	#[test]
	fn app_declarations_are_kept_and_not_repeated() {
		let source = android().replace(
			"<application",
			"<uses-feature android:name=\"android.hardware.vr.headtracking\" \
			 android:required=\"false\"/>\n<queries><package \
			 android:name=\"com.example\"/></queries>\n<application",
		);
		let manifest = manifest(&source, &QuestConfig::default()).unwrap();
		assert_eq!(
			manifest.matches("android.hardware.vr.headtracking").count(),
			1
		);
		assert_eq!(manifest.matches("<queries>").count(), 1);
		assert!(manifest.contains("<package android:name=\"com.example\"/><provider"));
	}

	#[test]
	fn a_manifest_without_a_launcher_is_refused() {
		let source = android().replace("android.intent.category.LAUNCHER", "example.OTHER");
		let error = manifest(&source, &QuestConfig::default()).unwrap_err();
		assert!(error.to_string().contains("launcher activity"), "{error}");
	}
}
