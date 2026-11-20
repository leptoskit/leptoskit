//! The Wear OS build: an Android host packaged for Wear OS smartwatches
//! (Pixel Watch, Samsung Galaxy Watch / One UI Watch, `TicWatch`, etc.).
//!
//! Wear OS runs Android APKs, so `--platform wearos` builds the Android host
//! (app-owned `platforms/wearos` if present, or `platforms/android`) and
//! transforms its merged `AndroidManifest.xml` to declare:
//! - `<uses-feature android:name="android.hardware.type.watch" />`
//! - `<meta-data android:name="com.google.android.wearable.standalone" ... />`
//! - Optional Wear Tile and Complication data-source services when enabled.

use std::fmt::Write as _;
use std::ops::Range;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::WearosConfig;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";

type Edit = (Range<usize>, String);

/// Transforms an Android manifest (after plugin merging) for Wear OS.
pub fn manifest(source: &str, wearos: &WearosConfig) -> Result<String> {
	let document = roxmltree::Document::parse(source).context("invalid AndroidManifest.xml")?;
	let root = document.root_element();
	let prefix = root
		.lookup_prefix(ANDROID)
		.context("Android manifest must declare the Android attribute namespace")?;
	let application = root
		.children()
		.find(|node| node.has_tag_name("application"))
		.context("Android manifest has no application element")?;

	let mut edits: Vec<Edit> = Vec::new();
	let app_start = application.range().start;
	let feature_xml = watch_feature(root, prefix)?;
	if !feature_xml.is_empty() {
		edits.push((app_start..app_start, feature_xml));
	}

	let metadata_xml = standalone_metadata(application, prefix, wearos.standalone)?;
	if !metadata_xml.is_empty() {
		let at = application
			.first_child()
			.context("the Android manifest's application element must not be empty")?
			.range()
			.start;
		edits.push((at..at, metadata_xml));
	}

	let services_xml = watch_services(application, prefix, wearos)?;
	if !services_xml.is_empty() {
		let close = source[..application.range().end]
			.rfind("</application>")
			.filter(|close| *close > application.range().start)
			.context("the Android manifest's application element must not be self-closing")?;
		edits.push((close..close, services_xml));
	}

	edits.sort_by_key(|edit| std::cmp::Reverse((edit.0.start, edit.0.end)));
	let mut output = source.to_owned();
	for (range, replacement) in edits {
		output.replace_range(range, &replacement);
	}

	roxmltree::Document::parse(&output).context("generated Wear OS manifest is invalid")?;
	Ok(output)
}

fn watch_feature(root: roxmltree::Node<'_, '_>, prefix: &str) -> Result<String> {
	let exists = root.children().any(|node| {
		node.has_tag_name("uses-feature")
			&& node.attribute((ANDROID, "name")) == Some("android.hardware.type.watch")
	});
	if exists {
		return Ok(String::new());
	}
	let mut out = String::new();
	writeln!(
		out,
		"<uses-feature {prefix}:name=\"android.hardware.type.watch\" {prefix}:required=\"true\"/>"
	)?;
	Ok(out)
}

fn standalone_metadata(
	application: roxmltree::Node<'_, '_>,
	prefix: &str,
	standalone: bool,
) -> Result<String> {
	let exists = application.children().any(|node| {
		node.has_tag_name("meta-data")
			&& node.attribute((ANDROID, "name")) == Some("com.google.android.wearable.standalone")
	});
	if exists {
		return Ok(String::new());
	}
	let mut out = String::new();
	write!(
		out,
		"\n<meta-data {prefix}:name=\"com.google.android.wearable.standalone\" \
		 {prefix}:value=\"{standalone}\"/>"
	)?;
	Ok(out)
}

fn watch_services(
	application: roxmltree::Node<'_, '_>,
	prefix: &str,
	wearos: &WearosConfig,
) -> Result<String> {
	let has_service = |name: &str| {
		application.children().any(|node| {
			node.has_tag_name("service") && node.attribute((ANDROID, "name")) == Some(name)
		})
	};
	let mut out = String::new();
	if wearos.tile && !has_service("dev.leptoskit.wearos.LeptosKitTileService") {
		writeln!(
			out,
			"<service {prefix}:name=\"dev.leptoskit.wearos.LeptosKitTileService\" \
			 {prefix}:exported=\"true\" \
			 {prefix}:permission=\"com.google.android.wearable.permission.BIND_TILE_PROVIDER\"\
			 ><intent-filter><action \
			 {prefix}:name=\"androidx.wear.tiles.action.BIND_TILE_PROVIDER\"/></intent-filter></\
			 service>"
		)?;
	}
	if wearos.complication && !has_service("dev.leptoskit.wearos.LeptosKitComplicationDataSource") {
		writeln!(
			out,
			"<service {prefix}:name=\"dev.leptoskit.wearos.LeptosKitComplicationDataSource\" \
			 {prefix}:exported=\"true\" \
			 {prefix}:permission=\"com.google.android.wearable.permission.\
			 BIND_COMPLICATION_PROVIDER\"><intent-filter><action \
			 {prefix}:name=\"android.support.wearable.complications.\
			 ACTION_COMPLICATION_UPDATE_REQUEST\"/></intent-filter><meta-data \
			 {prefix}:name=\"android.support.wearable.complications.SUPPORTED_TYPES\" \
			 {prefix}:value=\"SHORT_TEXT,RANGED_VALUE\"/><meta-data \
			 {prefix}:name=\"android.support.wearable.complications.UPDATE_PERIOD_SECONDS\" \
			 {prefix}:value=\"300\"/></service>"
		)?;
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application android:label="Quant">
        <activity android:name=".MainActivity" android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#;

	#[test]
	fn adds_watch_feature_standalone_metadata_and_services() {
		let transformed = manifest(SAMPLE, &WearosConfig::default()).unwrap();
		assert!(transformed.contains("android.hardware.type.watch"));
		assert!(transformed.contains("com.google.android.wearable.standalone"));
		assert!(transformed.contains("LeptosKitTileService"));
		assert!(transformed.contains("LeptosKitComplicationDataSource"));
		// Idempotent when applied twice.
		let twice = manifest(&transformed, &WearosConfig::default()).unwrap();
		assert_eq!(twice, transformed);
	}

	#[test]
	fn respects_optional_tile_and_complication_flags() {
		let cfg = WearosConfig {
			standalone: false,
			tile: false,
			complication: false,
			samsung_health_sensor: true,
		};
		let transformed = manifest(SAMPLE, &cfg).unwrap();
		assert!(transformed.contains("android:value=\"false\""));
		assert!(!transformed.contains("LeptosKitTileService"));
		assert!(!transformed.contains("LeptosKitComplicationDataSource"));
	}
}
