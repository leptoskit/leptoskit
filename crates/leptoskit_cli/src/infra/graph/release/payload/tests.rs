#![cfg(unix)]
use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use serde_json::json;

use super::*;

#[test]
fn services_without_assets_transport_exact_backend_and_enforce_budget() {
	let directory = tempfile::tempdir().unwrap();
	let unit = directory.path().join("services.api");
	fs::create_dir(&unit).unwrap();
	fs::write(unit.join("backend"), b"backend").unwrap();
	fs::set_permissions(unit.join("backend"), fs::Permissions::from_mode(0o755)).unwrap();
	let source = Root::open(directory.path()).unwrap();
	let host = json!({"units":{"services.api":{}},"executables":{"services.api":{
		"backend":{"sha256":super::super::hash(b"backend"),"size":7,"mode":0o755}}}});
	let value = read(&source, &host, 0).unwrap();
	assert!(value.files.is_empty());
	assert_eq!(
		value.backends["services.api"],
		super::super::encode(b"backend")
	);
	assert!(read(&source, &host, LIMIT - 6).is_err());
	fs::write(unit.join("backend"), b"changed").unwrap();
	assert!(read(&source, &host, 0).is_err());
}

#[test]
fn static_assets_and_backend_share_one_transport_budget() {
	let directory = tempfile::tempdir().unwrap();
	let assets = directory.path().join("frontends.site/assets");
	fs::create_dir_all(&assets).unwrap();
	fs::write(assets.join("index.html"), b"page").unwrap();
	let source = Root::open(directory.path()).unwrap();
	let host = json!({"units":{"frontends.site":{"index.html":{"kind":"file","size":4}}}});
	assert_eq!(read(&source, &host, LIMIT - 4).unwrap().files.len(), 1);
	assert!(read(&source, &host, LIMIT - 3).is_err());
}
