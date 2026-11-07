//! Package visibility and activities that selected plugins declare.
//!
//! The app's own declarations win: a package it already queries, or an
//! activity or alias it already declares under the same name, is left as
//! the app wrote it.
use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::Context;
use anyhow::Result;
use roxmltree::Node;

use super::ANDROID;
use super::Edit;
use crate::native::registry::AndroidActivity;

pub fn merge(
	source: &str,
	packages: &BTreeSet<&str>,
	activities: &[&AndroidActivity],
) -> Result<String> {
	if packages.is_empty() && activities.is_empty() {
		return Ok(source.to_owned());
	}

	let document = roxmltree::Document::parse(source).context("invalid AndroidManifest.xml")?;
	let root = document.root_element();
	let prefix = root
		.lookup_prefix(ANDROID)
		.context("Android manifest must declare the Android attribute namespace")?;
	let application = root
		.children()
		.find(|node| node.has_tag_name("application"))
		.context("Android manifest has no application element")?;
	let queries: Vec<_> = root
		.children()
		.filter(|node| node.has_tag_name("queries"))
		.collect();
	anyhow::ensure!(
		queries.len() <= 1,
		"Android manifest must contain at most one queries element"
	);
	let mut edits: Vec<Edit> = Vec::new();
	let missing = missing_packages(queries.first().copied(), packages, prefix)?;

	if let Some(edit) = super::queries::insertion(
		source,
		queries.first().copied(),
		application.range().start,
		missing,
	)? {
		edits.push(edit);
	}

	let markup = missing_activities(application, activities, prefix)?;

	if !markup.is_empty() {
		edits.push(append(source, application, markup)?);
	}

	edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
	let mut output = source.to_owned();

	for (range, replacement) in edits {
		output.replace_range(range, &replacement);
	}

	roxmltree::Document::parse(&output).context("generated Android manifest is invalid")?;
	Ok(output)
}

fn missing_packages(
	queries: Option<Node<'_, '_>>,
	packages: &BTreeSet<&str>,
	prefix: &str,
) -> Result<String> {
	let declared: BTreeSet<_> = queries
		.into_iter()
		.flat_map(|queries| queries.children())
		.filter(|node| node.has_tag_name("package"))
		.filter_map(|node| node.attribute((ANDROID, "name")))
		.collect();
	let mut output = String::new();

	for package in packages.difference(&declared) {
		writeln!(output, "<package {prefix}:name=\"{package}\"/>")?;
	}

	Ok(output)
}

/// Activities come before aliases, because an alias must follow the
/// activity it targets.
fn missing_activities(
	application: Node<'_, '_>,
	activities: &[&AndroidActivity],
	prefix: &str,
) -> Result<String> {
	let declared: BTreeSet<_> = application
		.children()
		.filter(|node| node.has_tag_name("activity") || node.has_tag_name("activity-alias"))
		.filter_map(|node| node.attribute((ANDROID, "name")))
		.collect();
	let mut written = BTreeSet::new();
	let mut output = String::new();
	let ordered = activities
		.iter()
		.filter(|activity| activity.target_activity.is_none())
		.chain(
			activities
				.iter()
				.filter(|activity| activity.target_activity.is_some()),
		);

	for activity in ordered {
		if declared.contains(activity.name.as_str()) || !written.insert(activity.name.as_str()) {
			continue;
		}

		write_activity(&mut output, activity, prefix)?;
	}

	Ok(output)
}

fn write_activity(output: &mut String, activity: &AndroidActivity, prefix: &str) -> Result<()> {
	let element = if activity.target_activity.is_some() {
		"activity-alias"
	} else {
		"activity"
	};
	write!(
		output,
		"<{element} {prefix}:name=\"{}\" {prefix}:exported=\"{}\"",
		activity.name, activity.exported
	)?;

	for (attribute, value) in [
		("targetActivity", &activity.target_activity),
		("permission", &activity.permission),
		("theme", &activity.theme),
	] {
		if let Some(value) = value {
			write!(output, " {prefix}:{attribute}=\"{value}\"")?;
		}
	}

	output.push('>');

	for filter in &activity.intent_filters {
		output.push_str("<intent-filter>");

		for action in &filter.actions {
			write!(output, "<action {prefix}:name=\"{action}\"/>")?;
		}

		for category in &filter.categories {
			write!(output, "<category {prefix}:name=\"{category}\"/>")?;
		}

		output.push_str("</intent-filter>");
	}

	writeln!(output, "</{element}>")?;
	Ok(())
}

/// Appends `markup` as the element's last children, opening a self-closing
/// element.
fn append(source: &str, element: Node<'_, '_>, markup: String) -> Result<Edit> {
	let range = element.range();
	let text = &source[range.clone()];

	if text.ends_with("/>") {
		let name = element.tag_name().name();
		return Ok((range.end - 2..range.end, format!(">\n{markup}</{name}>")));
	}

	let closing = range.start + text.rfind("</").context("closing element missing")?;
	Ok((closing..closing, markup))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn activity(value: serde_json::Value) -> AndroidActivity {
		serde_json::from_value(value).unwrap()
	}

	fn rationale() -> [AndroidActivity; 2] {
		[
			activity(serde_json::json!({
				"name": "dev.example.ViewUsage",
				"target-activity": "dev.example.Rationale",
				"exported": true,
				"permission": "android.permission.START_VIEW_PERMISSION_USAGE",
				"intent-filters": [{
					"actions": ["android.intent.action.VIEW_PERMISSION_USAGE"],
					"categories": ["android.intent.category.HEALTH_PERMISSIONS"]
				}]
			})),
			activity(serde_json::json!({
				"name": "dev.example.Rationale",
				"exported": true,
				"theme": "@android:style/Theme.NoDisplay",
				"intent-filters": [{"actions": ["androidx.health.ACTION_SHOW_PERMISSIONS_RATIONALE"]}]
			})),
		]
	}

	#[test]
	fn adds_packages_and_activities_with_aliases_after_their_targets() {
		let source = r#"<manifest xmlns:a="http://schemas.android.com/apk/res/android"><application a:label="App"/></manifest>"#;
		let activities = rationale();
		let references: Vec<_> = activities.iter().collect();
		let packages = BTreeSet::from(["com.google.android.apps.healthdata"]);
		let output = merge(source, &packages, &references).unwrap();
		let document = roxmltree::Document::parse(&output).unwrap();
		let package = document
			.descendants()
			.find(|node| node.has_tag_name("package"))
			.unwrap();
		assert_eq!(
			package.attribute((ANDROID, "name")),
			Some("com.google.android.apps.healthdata")
		);
		assert_eq!(package.parent().unwrap().tag_name().name(), "queries");
		let application = document
			.descendants()
			.find(|node| node.has_tag_name("application"))
			.unwrap();
		let children: Vec<_> = application.children().filter(Node::is_element).collect();
		assert_eq!(children[0].tag_name().name(), "activity");
		assert_eq!(
			children[0].attribute((ANDROID, "theme")),
			Some("@android:style/Theme.NoDisplay")
		);
		assert_eq!(children[1].tag_name().name(), "activity-alias");
		assert_eq!(
			children[1].attribute((ANDROID, "targetActivity")),
			Some("dev.example.Rationale")
		);
		assert_eq!(children[1].attribute((ANDROID, "exported")), Some("true"));
		let category = children[1]
			.descendants()
			.find(|node| node.has_tag_name("category"))
			.unwrap();
		assert_eq!(
			category.attribute((ANDROID, "name")),
			Some("android.intent.category.HEALTH_PERMISSIONS")
		);
		assert_eq!(merge(&output, &packages, &references).unwrap(), output);
	}

	#[test]
	fn keeps_the_apps_own_declarations_and_comments() {
		let source = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android">
<queries><package android:name="com.google.android.apps.healthdata"/></queries>
<application>
<!-- the app's own rationale screen -->
<activity android:name="dev.example.Rationale" android:exported="true"/>
</application>
</manifest>"#;
		let activities = rationale();
		let references: Vec<_> = activities.iter().collect();
		let packages = BTreeSet::from(["com.google.android.apps.healthdata"]);
		let output = merge(source, &packages, &references).unwrap();
		assert_eq!(
			output.matches("com.google.android.apps.healthdata").count(),
			1
		);
		assert_eq!(output.matches("dev.example.Rationale\"").count(), 2);
		assert!(output.contains("<!-- the app's own rationale screen -->"));
		assert!(output.contains("<activity-alias android:name=\"dev.example.ViewUsage\""));
		assert!(!output.contains("Theme.NoDisplay"));
	}

	#[test]
	fn nothing_selected_leaves_the_manifest_alone_and_bad_xml_fails() {
		assert_eq!(merge("not xml", &BTreeSet::new(), &[]).unwrap(), "not xml");
		assert!(merge("<manifest>", &BTreeSet::from(["a.b"]), &[]).is_err());
		let doubled = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android"><queries/><queries/><application/></manifest>"#;
		assert!(merge(doubled, &BTreeSet::from(["a.b"]), &[]).is_err());
	}
}
