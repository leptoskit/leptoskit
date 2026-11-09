use super::*;

#[test]
fn serialized_member_contract_matches_existing_website_descriptors() {
	let member = Member::File {
		mode: 0o644,
		size: 1,
		sha256: "a".repeat(64),
	};
	assert_eq!(
		serde_json::to_value(&member).unwrap(),
		serde_json::json!({
			"kind":"file", "mode":420, "size":1, "sha256":"a".repeat(64)
		})
	);
	assert_eq!(
		serde_json::from_value::<Member>(serde_json::to_value(&member).unwrap()).unwrap(),
		member
	);
}

#[test]
fn membership_requires_parents_modes_digests_and_bounds() {
	let file = Member::File {
		mode: 0o644,
		size: 1,
		sha256: "a".repeat(64),
	};
	let mut tree = Tree::from([("pkg/app.js".into(), file)]);
	assert!(validate(&tree).is_err());
	tree.insert("pkg".into(), Member::Directory { mode: 0o755 });
	validate(&tree).unwrap();

	for invalid in ["../secret", "/root", "a\\b", "a//b", "./name", "a\nb"] {
		assert!(!safe_path(invalid), "{invalid}");
	}
	for (mode, size, digest) in [
		(0o755, 1, "a".repeat(64)),
		(0o644, MAX_BYTES + 1, "a".repeat(64)),
		(0o644, 1, "A".repeat(64)),
	] {
		tree.insert(
			"pkg/app.js".into(),
			Member::File {
				mode,
				size,
				sha256: digest,
			},
		);
		assert!(validate(&tree).is_err());
	}
}

#[cfg(unix)]
mod filesystem {
	use std::fs;
	use std::os::unix::fs::PermissionsExt as _;
	use std::os::unix::fs::symlink;

	use super::*;

	fn directory(path: &Path) {
		fs::create_dir_all(path).unwrap();
		fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
	}

	fn fixture() -> tempfile::TempDir {
		let temp = tempfile::tempdir().unwrap();

		for path in ["out", "out/docs", "out/docs/pkg", "out/play"] {
			directory(&temp.path().join(path));
		}
		for (path, body) in [
			("out/index.html", "landing"),
			("out/docs/index.html", "docs"),
			("out/docs/pkg/app.wasm", "wasm"),
			("out/play/frame.mjs", "lazy"),
		] {
			fs::write(temp.path().join(path), body).unwrap();
			fs::set_permissions(temp.path().join(path), fs::Permissions::from_mode(0o644)).unwrap();
		}
		temp
	}

	#[test]
	fn shared_output_is_partitioned_without_copying_unselected_subtrees() {
		let temp = fixture();
		let project = Root::open(temp.path()).unwrap();
		let output = project.beneath("out").unwrap();
		let excluded = ["docs".into(), "play".into()];
		let site = output.tree(&excluded).unwrap();
		assert_eq!(
			site.keys().map(String::as_str).collect::<Vec<_>>(),
			["index.html"]
		);
		output
			.copy(&temp.path().join("site-capture"), &excluded, &site)
			.unwrap();
		let docs = output.beneath("docs").unwrap();

		let tree = docs.tree(&[]).unwrap();
		docs.copy(&temp.path().join("docs-capture"), &[], &tree)
			.unwrap();
		assert_eq!(
			fs::read_to_string(temp.path().join("docs-capture/index.html")).unwrap(),
			"docs"
		);
		assert!(!temp.path().join("docs-capture/play").exists());
		assert_eq!(
			Root::open(&temp.path().join("docs-capture"))
				.unwrap()
				.tree(&[])
				.unwrap(),
			tree
		);
	}

	#[test]
	fn mutation_after_inventory_refuses_copy_before_destination_creation() {
		let temp = fixture();
		let root = Root::open(&temp.path().join("out")).unwrap();
		let tree = root.tree(&[]).unwrap();
		fs::write(temp.path().join("out/index.html"), "changed").unwrap();
		assert!(root.copy(&temp.path().join("capture"), &[], &tree).is_err());
		assert!(!temp.path().join("capture").exists());
	}

	#[test]
	fn changed_membership_and_modes_refuse_copy() {
		for changed_mode in [false, true] {
			let temp = fixture();
			let root = Root::open(&temp.path().join("out")).unwrap();
			let tree = root.tree(&[]).unwrap();

			if changed_mode {
				fs::set_permissions(
					temp.path().join("out/index.html"),
					fs::Permissions::from_mode(0o755),
				)
				.unwrap();
			} else {
				fs::write(temp.path().join("out/extra"), "unexpected").unwrap();
			}

			assert!(root.copy(&temp.path().join("capture"), &[], &tree).is_err());
		}
	}

	#[test]
	fn symlinks_are_not_artifact_members_or_output_roots() {
		let temp = fixture();
		symlink("docs", temp.path().join("out/link")).unwrap();
		let root = Root::open(&temp.path().join("out")).unwrap();
		assert!(root.tree(&[]).is_err());
		assert!(root.beneath("link").is_err());
		assert!(Root::open(&temp.path().join("out/link")).is_err());
		// An independently owned subtree is omitted altogether.
		root.tree(&["link".into()]).unwrap();
	}

	#[test]
	fn root_handle_keeps_its_identity_when_its_path_is_renamed() {
		let temp = fixture();
		let root = Root::open(&temp.path().join("out")).unwrap();
		let before = root.tree(&[]).unwrap();
		fs::rename(temp.path().join("out"), temp.path().join("previous")).unwrap();
		directory(&temp.path().join("out"));
		fs::write(temp.path().join("out/index.html"), "different root").unwrap();
		assert_eq!(root.tree(&[]).unwrap(), before);
		root.copy(&temp.path().join("capture"), &[], &before)
			.unwrap();

		assert_eq!(
			fs::read_to_string(temp.path().join("capture/index.html")).unwrap(),
			"landing"
		);
	}

	#[test]
	fn copy_never_replaces_an_existing_destination() {
		let temp = fixture();
		let root = Root::open(&temp.path().join("out")).unwrap();
		let expected = root.tree(&[]).unwrap();
		let destination = temp.path().join("capture");
		directory(&destination);
		fs::write(destination.join("keep"), "operator data").unwrap();
		assert!(root.copy(&destination, &[], &expected).is_err());
		assert_eq!(
			fs::read_to_string(destination.join("keep")).unwrap(),
			"operator data"
		);
	}
}
