//! Merge plugin-declared application components while preserving app-owned XML.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::Context;
use anyhow::Result;
use roxmltree::Node;

use super::ANDROID;
use super::Edit;

/// A plugin-declared Android `<service>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceComponent {
	pub name: String,
	pub foreground_service_type: String,
	pub exported: bool,
}

/// A plugin-declared Android `<receiver>` with its intent-filter actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverComponent {
	pub name: String,
	pub exported: bool,
	pub actions: Vec<String>,
}

/// Inserts missing `<service>` and `<receiver>` elements into the application,
/// matched by `android:name` so an app-owned declaration wins.
pub fn app_components(
	source: &str,
	services: &[ServiceComponent],
	receivers: &[ReceiverComponent],
) -> Result<String> {
	if services.is_empty() && receivers.is_empty() {
		return Ok(source.to_owned());
	}

	let document = roxmltree::Document::parse(source).context("invalid AndroidManifest.xml")?;
	let root = document.root_element();
	anyhow::ensure!(
		root.has_tag_name("manifest"),
		"Android XML root must be manifest"
	);
	let prefix = root
		.lookup_prefix(ANDROID)
		.context("Android manifest must declare the Android attribute namespace")?;
	let application = root
		.children()
		.find(|node| node.has_tag_name("application"))
		.context("Android manifest has no application element")?;

	let Some(edit) = insertion(source, application, services, receivers, prefix)? else {
		return Ok(source.to_owned());
	};
	let (range, replacement) = edit;
	let mut output = source.to_owned();
	output.replace_range(range, &replacement);

	roxmltree::Document::parse(&output).context("generated Android manifest is invalid")?;
	Ok(output)
}

fn insertion(
	source: &str,
	application: Node<'_, '_>,
	services: &[ServiceComponent],
	receivers: &[ReceiverComponent],
	prefix: &str,
) -> Result<Option<Edit>> {
	let declared: BTreeSet<_> = application
		.children()
		.filter(|node| node.has_tag_name("service") || node.has_tag_name("receiver"))
		.filter_map(|node| node.attribute((ANDROID, "name")))
		.collect();
	let mut declared = declared;
	let mut additions = String::new();

	for service in services {
		if !declared.insert(service.name.as_str()) {
			continue;
		}

		if service.foreground_service_type.is_empty() {
			writeln!(
				additions,
				"<service {prefix}:name=\"{}\" {prefix}:exported=\"{}\"/>",
				service.name, service.exported
			)?;
		} else {
			writeln!(
				additions,
				"<service {prefix}:name=\"{}\" {prefix}:exported=\"{}\" \
				 {prefix}:foregroundServiceType=\"{}\"/>",
				service.name, service.exported, service.foreground_service_type
			)?;
		}
	}

	for receiver in receivers {
		if !declared.insert(receiver.name.as_str()) {
			continue;
		}

		if receiver.actions.is_empty() {
			writeln!(
				additions,
				"<receiver {prefix}:name=\"{}\" {prefix}:exported=\"{}\"/>",
				receiver.name, receiver.exported
			)?;
		} else {
			writeln!(
				additions,
				"<receiver {prefix}:name=\"{}\" {prefix}:exported=\"{}\">",
				receiver.name, receiver.exported
			)?;

			for action in &receiver.actions {
				writeln!(
					additions,
					"<intent-filter><action {prefix}:name=\"{action}\"/></intent-filter>"
				)?;
			}

			additions.push_str("</receiver>\n");
		}
	}

	if additions.is_empty() {
		return Ok(None);
	}

	let range = application.range();
	let element = &source[range.clone()];

	if element.ends_with("/>") {
		return Ok(Some((
			range.end - 2..range.end,
			format!(">\n{additions}</application>"),
		)));
	}

	let closing = range.start
		+ element
			.rfind("</")
			.context("application closing element missing")?;
	Ok(Some((closing..closing, additions)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn inserts_missing_components_and_preserves_app_owned_ones() {
		let source = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android"><application android:name=".App"><activity android:name=".Main"/></application></manifest>"#;
		let services = [ServiceComponent {
			name: "dev.leptoskit.plugins.background.BackgroundService".into(),
			foreground_service_type: "dataSync|health".into(),
			exported: false,
		}];
		let receivers = [ReceiverComponent {
			name: "dev.leptoskit.plugins.background.BootReceiver".into(),
			exported: false,
			actions: vec!["android.intent.action.BOOT_COMPLETED".into()],
		}];
		let output = app_components(source, &services, &receivers).unwrap();

		assert!(output.contains(
			"<service android:name=\"dev.leptoskit.plugins.background.BackgroundService\" \
			 android:exported=\"false\" android:foregroundServiceType=\"dataSync|health\"/>"
		));
		assert!(output.contains(
			"<receiver android:name=\"dev.leptoskit.plugins.background.BootReceiver\" \
			 android:exported=\"false\">"
		));
		assert!(output.contains("<action android:name=\"android.intent.action.BOOT_COMPLETED\"/>"));
		assert!(output.contains("<activity android:name=\".Main\"/>"));

		// Idempotent: a second merge adds nothing.
		assert_eq!(
			app_components(&output, &services, &receivers).unwrap(),
			output
		);
	}

	#[test]
	fn respects_app_owned_declarations_and_self_closing_application() {
		let source = r#"<manifest xmlns:a="http://schemas.android.com/apk/res/android"><application a:name=".App"><service a:name="dev.leptoskit.plugins.background.BackgroundService" a:exported="false"/></application></manifest>"#;
		let services = [ServiceComponent {
			name: "dev.leptoskit.plugins.background.BackgroundService".into(),
			foreground_service_type: "dataSync".into(),
			exported: true,
		}];
		let output = app_components(source, &services, &[]).unwrap();
		assert_eq!(output, source);

		let self_closing = r#"<manifest xmlns:a="http://schemas.android.com/apk/res/android"><application a:name=".App"/></manifest>"#;
		let output = app_components(self_closing, &services, &[]).unwrap();
		assert!(
			output
				.contains("<service a:name=\"dev.leptoskit.plugins.background.BackgroundService\"")
		);
		assert!(output.contains("</application>"));
	}
}
