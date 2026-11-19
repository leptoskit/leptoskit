//! The Solana dApp Store, for Solana Mobile's Seeker and Saga phones: the
//! release-signed Android APK, published by Solana Mobile's official
//! `@solana-mobile/dapp-store-cli`, whose one invocation ingests the APK,
//! mints the release NFT, and submits the release for review.
//!
//! The tool has no draft stage, so without `--submit` nothing is sent: the
//! signed APK is built, every check runs, and the run stops. The app is
//! matched by the Android package name inside the APK, and must already
//! exist in the Publisher Portal.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use base64::Engine as _;
use serde_json::json;

use super::http::Body;
use super::http::Http;
use super::secrets::Credentials;
use super::secrets::Secret;
use super::tools::Tool;
use crate::native::signing::KeystoreVars;

/// The portal API key, which the tool reads from its own environment.
pub const API_KEY: &str = "LEPTOSKIT_PUBLISH_SOLANA_API_KEY";
/// Where to make it.
pub const API_KEY_HOW: &str = "sign in at the Publisher Portal (https://publish.solanamobile.com) \
                               with the account that holds the app, and create a key under \
                               Settings, API keys";
/// The Solana CLI keypair of the publisher wallet, as a file's path, or the
/// JSON array itself.
pub const KEYPAIR: &str = "LEPTOSKIT_PUBLISH_SOLANA_KEYPAIR";
/// Where to make it.
pub const KEYPAIR_HOW: &str = "make a wallet dedicated to publishing with `solana-keygen new \
                               --no-bip39-passphrase --outfile publish-keypair.json`, keep it \
                               outside the project, and set this to the file's path, or to the \
                               JSON array itself";
/// The keystore the APK is signed with, separate from Google Play's: the
/// dApp Store refuses the key Google Play signs with.
pub const KEYSTORE: &str = "LEPTOSKIT_SOLANA_KEYSTORE";
/// The keystore itself, base64-encoded, in place of its path.
pub const KEYSTORE_BASE64: &str = "LEPTOSKIT_SOLANA_KEYSTORE_BASE64";
/// The keystore's password.
pub const KEYSTORE_PASSWORD: &str = "LEPTOSKIT_SOLANA_KEYSTORE_PASSWORD";
/// The signing key's alias in the keystore.
pub const KEY_ALIAS: &str = "LEPTOSKIT_SOLANA_KEY_ALIAS";
/// The signing key's password.
pub const KEY_PASSWORD: &str = "LEPTOSKIT_SOLANA_KEY_PASSWORD";
/// Where to make one.
pub const KEYSTORE_HOW: &str = "make a keystore for the dApp Store alone with `keytool \
                                -genkeypair -keystore dappstore.jks -alias dappstore -keyalg RSA \
                                -keysize 2048 -validity 10000` — the dApp Store refuses the key \
                                Google Play signs with — keep it outside the project, and set its \
                                absolute path, its two passwords, and its alias";

/// The keystore variables an Android release for this store is signed
/// with, read by the same code as Google Play's upload key.
pub(crate) const KEYSTORE_VARS: KeystoreVars = KeystoreVars {
	keystore: KEYSTORE,
	keystore_base64: KEYSTORE_BASE64,
	keystore_password: KEYSTORE_PASSWORD,
	key_alias: KEY_ALIAS,
	key_password: KEY_PASSWORD,
	how: KEYSTORE_HOW,
};

/// The environment variable the tool reads the portal API key from.
pub const API_KEY_ENV: &str = "DAPP_STORE_API_KEY";
/// Solana Mobile's official publishing tool.
pub const CLI: &str = "@solana-mobile/dapp-store-cli";
/// The RPC address the wallet's balance is read from, when
/// `[publish.solana].rpc_url` isn't set: Solana Labs' public mainnet RPC.
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
/// The RPC address override, for a proxy or a test server. Whatever address
/// this names receives the wallet's address, so only set it deliberately.
pub const RPC_OVERRIDE: &str = "LEPTOSKIT_PUBLISH_SOLANA_RPC";
/// The smallest wallet balance the tool starts with: minting the release
/// NFT needs about 0.015 SOL of rent plus fees.
pub const MIN_BALANCE_LAMPORTS: u64 = 16_000_000;
/// The ABI the store requires the APK to support.
pub const REQUIRED_ABI: &str = "arm64-v8a";

/// What Gradle signs the APK with, taken from the dApp Store keystore.
pub struct Signing {
	/// The keystore's path.
	pub keystore: Secret,
	pub keystore_password: Secret,
	pub key_alias: Secret,
	pub key_password: Secret,
}

/// `gradlew assembleRelease` in the Android host, signed with the dApp
/// Store keystore. The generated script reads the four `LEPTOSKIT_ANDROID_*`
/// variables, so the Solana values go to Gradle under those names, in this
/// one tool's environment — never the process environment, where a later
/// Google Play build would read them.
#[must_use]
pub fn apk(android: &Path, signing: &Signing, build_number: Option<u64>) -> Tool {
	let mut tool = Tool::new(
		&android
			.join(if cfg!(windows) {
				"gradlew.bat"
			} else {
				"gradlew"
			})
			.display()
			.to_string(),
	)
	.dir(android)
	.args([
		"--no-daemon",
		"--console=plain",
		"--quiet",
		"assembleRelease",
	]);

	if let Some(build_number) = build_number {
		tool = tool.arg(format!("-Pleptoskit.versionCode={build_number}"));
	}

	tool.env(
		crate::native::signing::ANDROID_KEYSTORE,
		signing.keystore.clone(),
	)
	.env(
		crate::native::signing::ANDROID_KEYSTORE_PASSWORD,
		signing.keystore_password.clone(),
	)
	.env(
		crate::native::signing::ANDROID_KEY_ALIAS,
		signing.key_alias.clone(),
	)
	.env(
		crate::native::signing::ANDROID_KEY_PASSWORD,
		signing.key_password.clone(),
	)
}

/// The tool's one invocation, which ingests the APK, mints the release NFT,
/// and submits the release for review. The portal API key rides only in
/// this tool's environment.
#[must_use]
pub fn submit(
	apk: &Path,
	keypair: &KeypairFile,
	whats_new: &str,
	api_key: Secret,
	cli_version: &str,
) -> Tool {
	Tool::new(if cfg!(windows) { "npx.cmd" } else { "npx" })
		.args(["--yes", &format!("{CLI}@{cli_version}")])
		.arg("--apk-file")
		.path(apk)
		.arg("--keypair")
		.path(keypair.path())
		.args(["--whats-new", whats_new])
		.env(API_KEY_ENV, api_key)
}

/// A Solana CLI keypair, held as the file the tool reads: the credential's
/// own file when it named one, or the JSON in a private temporary file,
/// removed on drop.
pub struct KeypairFile {
	path: PathBuf,
	keypair: [u8; 64],
	/// The private directory a JSON credential was written to.
	_temporary: Option<tempfile::TempDir>,
}

impl KeypairFile {
	/// Reads the keypair credential: a path to a Solana CLI keypair JSON
	/// file, or the JSON array itself. Either must hold a 64-byte keypair.
	///
	/// # Errors
	///
	/// When the file is unreadable, inside the project and not git-ignored,
	/// or neither form parses as a keypair.
	pub fn resolve(root: &Path, credential: &Secret) -> Result<Self> {
		let value = credential.expose().trim();

		if let Some(keypair) = json_keypair(value) {
			let temporary = tempfile::Builder::new()
				.prefix("leptoskit-keypair-")
				.tempdir()
				.context("could not make a private temporary directory for the keypair")?;
			let path = temporary.path().join("publish-keypair.json");
			crate::native::signing::write_private(&path, value.as_bytes())?;

			return Ok(Self {
				path,
				keypair,
				_temporary: Some(temporary),
			});
		}

		super::secrets::refuse_path(KEYPAIR)?;
		let path = PathBuf::from(value);
		anyhow::ensure!(
			path.is_file(),
			"{KEYPAIR} names a file that doesn't exist or can't be read; set it to the keypair \
			 JSON file's path, or to the JSON array itself"
		);
		ignored_or_outside(root, &path)?;
		let text = std::fs::read_to_string(&path)
			.with_context(|| format!("could not read {KEYPAIR}'s file"))?;
		let keypair = json_keypair(text.trim()).with_context(|| {
			format!(
				"{KEYPAIR}'s file doesn't hold a Solana keypair: 64 numbers, as `solana-keygen \
				 new` writes them"
			)
		})?;

		Ok(Self {
			path: std::fs::canonicalize(&path)?,
			keypair,
			_temporary: None,
		})
	}

	/// The file the tool reads.
	pub fn path(&self) -> &Path {
		&self.path
	}

	/// The wallet's address: the keypair's public half, in base58, as
	/// wallets and explorers show it.
	#[must_use]
	pub fn address(&self) -> String {
		leptoskit_db::base58::encode(&self.keypair[32..])
	}
}

/// The 64 bytes of a Solana CLI keypair, when `text` is its JSON: the
/// 32-byte seed, then the 32-byte public key.
fn json_keypair(text: &str) -> Option<[u8; 64]> {
	let bytes: Vec<u8> = serde_json::from_str(text).ok()?;
	bytes.try_into().ok()
}

/// Refuses `path` inside the project unless git ignores it, so the wallet's
/// keypair can't be committed: the check `--credentials` files go through.
fn ignored_or_outside(root: &Path, path: &Path) -> Result<()> {
	let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
	let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

	if !absolute.starts_with(&root) {
		return Ok(());
	}

	let ignored = crate::git::command(&root)
		.args(["check-ignore", "-q"])
		.arg(&absolute)
		.stderr(std::process::Stdio::null())
		.status()
		.is_ok_and(|status| status.success());
	anyhow::ensure!(
		ignored,
		"{KEYPAIR} names a file inside the project that git doesn't ignore; move it out, or \
		 ignore it, so your wallet's keypair can't be committed"
	);
	Ok(())
}

/// The wallet's balance in lamports, from one JSON-RPC `getBalance` call.
///
/// # Errors
///
/// When the RPC can't be reached, or its answer has no balance.
pub fn balance(http: &Http, rpc_url: &str, address: &str) -> Result<u64> {
	let reply = http.send(
		"POST",
		rpc_url,
		&[("accept", "application/json")],
		Some(Body::json(&json!({
			"jsonrpc": "2.0",
			"id": 1,
			"method": "getBalance",
			"params": [address],
		}))),
	)?;
	let answer = reply
		.ok("reading the wallet's balance from the Solana RPC")?
		.json()?;
	answer["result"]["value"]
		.as_u64()
		.context("the Solana RPC answered without a balance")
}

/// Refuses the run when the dApp Store keystore and Google Play's upload
/// key are the same key, by content: the dApp Store refuses the APK
/// Google Play's key signs.
///
/// # Errors
///
/// When the two resolve to the same bytes, or a credential isn't readable.
pub fn refuse_the_play_keystore(root: &Path, credentials: &Credentials) -> Result<()> {
	let play = keystore_bytes(
		root,
		credentials.get(crate::publish::play::KEYSTORE),
		credentials.get(crate::native::signing::ANDROID_KEYSTORE_BASE64),
		crate::publish::play::KEYSTORE,
		crate::native::signing::ANDROID_KEYSTORE_BASE64,
	)?;
	let dapp_store = keystore_bytes(
		root,
		credentials.get(KEYSTORE),
		credentials.get(KEYSTORE_BASE64),
		KEYSTORE,
		KEYSTORE_BASE64,
	)?;

	if let (Some(play), Some(dapp_store)) = (play, dapp_store) {
		anyhow::ensure!(
			play != dapp_store,
			"{KEYSTORE} signs with Google Play's upload key; the dApp Store refuses the key \
			 Google Play signs with, and an APK signed with it is rejected. Make a dedicated \
			 keystore: {KEYSTORE_HOW}"
		);
	}

	Ok(())
}

/// The bytes a path-or-base64 keystore credential holds, or `None` when
/// neither form is set.
fn keystore_bytes(
	root: &Path,
	path: Option<Secret>,
	encoded: Option<Secret>,
	name: &str,
	base64_name: &str,
) -> Result<Option<Vec<u8>>> {
	anyhow::ensure!(
		path.is_none() || encoded.is_none(),
		"both {name} and {base64_name} are set; set only one"
	);

	if let Some(path) = path {
		super::secrets::refuse_path(name)?;
		let path = PathBuf::from(path.expose().trim());
		anyhow::ensure!(
			path.is_file(),
			"{name} names a file that doesn't exist or can't be read; set it to the keystore's \
			 absolute path"
		);
		crate::native::signing::outside_repository(root, &path, name, base64_name)?;
		return std::fs::read(&path)
			.map(Some)
			.with_context(|| format!("could not read {name}'s keystore"));
	}

	match encoded {
		None => Ok(None),
		Some(encoded) => {
			let encoded: String = encoded
				.expose()
				.chars()
				.filter(|character| !character.is_ascii_whitespace())
				.collect();
			// The decoder's error quotes the offending byte, which here is a
			// secret's.
			base64::engine::general_purpose::STANDARD
				.decode(encoded)
				.map(Some)
				.map_err(|_| anyhow::anyhow!("{base64_name} isn't base64"))
		}
	}
}

/// Refuses an x86-only Android build: the store takes an APK that supports
/// `arm64-v8a`. The generated build reads its one ABI from
/// `.leptoskit/android/abi.txt`, written when the Android host is prepared;
/// a project not prepared yet is checked again after that prepare, which
/// writes `arm64-v8a` for the phone build this store uploads.
///
/// # Errors
///
/// When `abi.txt` names an ABI other than [`REQUIRED_ABI`].
pub fn check_abis(root: &Path) -> Result<()> {
	let path = root.join(".leptoskit/android/abi.txt");
	let abi = match std::fs::read_to_string(&path) {
		Ok(abi) => Some(abi.trim().to_owned()),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
		Err(error) => {
			return Err(error).with_context(|| format!("could not read {}", path.display()));
		}
	};

	anyhow::ensure!(
		abi.as_deref().is_none_or(|abi| abi == REQUIRED_ABI),
		"{} names the Android build's only ABI `{}`; the dApp Store takes an APK that supports \
		 {REQUIRED_ABI}. Publish the phone build (`leptoskit native prepare android`), not an \
		 x86_64 emulator build",
		path.display(),
		abi.as_deref().unwrap_or_default()
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::publish::tests::json;
	use crate::publish::tests::store;

	/// A 64-number keypair, `solana-keygen new`'s JSON.
	fn keypair_json() -> String {
		let numbers: Vec<String> = (0..64).map(|index| index.to_string()).collect();
		format!("[{}]", numbers.join(","))
	}

	#[test]
	fn the_tool_gets_the_apk_the_keypair_and_the_key_alone() {
		let root = tempfile::tempdir().unwrap();
		let keypair = KeypairFile::resolve(root.path(), &Secret::new(keypair_json())).unwrap();
		let tool = submit(
			Path::new("/out/app-release.apk"),
			&keypair,
			"Faster sync.",
			Secret::new("portal-key".into()),
			"1.0.1",
		);

		assert_eq!(tool.program, "npx");
		assert_eq!(
			tool.args,
			[
				"--yes",
				"@solana-mobile/dapp-store-cli@1.0.1",
				"--apk-file",
				"/out/app-release.apk",
				"--keypair",
				keypair.path().display().to_string().as_str(),
				"--whats-new",
				"Faster sync.",
			]
		);
		// The key rides only here, and never prints with the tool.
		assert_eq!(tool.env.len(), 1);
		assert_eq!(tool.env[0].0, API_KEY_ENV);
		assert_eq!(tool.env[0].1.expose(), "portal-key");
		assert!(!format!("{tool:?}").contains("portal-key"), "{tool:?}");
	}

	#[test]
	fn a_json_keypair_is_parsed_and_written_privately() {
		let root = tempfile::tempdir().unwrap();
		let keypair = KeypairFile::resolve(root.path(), &Secret::new(keypair_json())).unwrap();
		assert!(!keypair.path().starts_with(root.path()));
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			let mode = std::fs::metadata(keypair.path())
				.unwrap()
				.permissions()
				.mode();
			assert_eq!(mode & 0o777, 0o600, "readable only by this user");
		}
		assert_eq!(
			keypair.address(),
			"3ARMH9zfVCnU2TKiphU4xcEyWdA45fc1sjKEtYMdf3gr",
			"the address is the public half, in base58"
		);
		drop(keypair);
	}

	#[test]
	fn a_keypair_that_is_not_64_numbers_is_refused() {
		let root = tempfile::tempdir().unwrap();
		let short = KeypairFile::resolve(root.path(), &Secret::new("[1,2,3]".into()))
			.err()
			.unwrap()
			.to_string();
		assert!(short.contains(KEYPAIR), "{short}");

		let not_json = KeypairFile::resolve(root.path(), &Secret::new("hunter2".into()))
			.err()
			.unwrap()
			.to_string();
		assert!(not_json.contains("doesn't exist"), "{not_json}");

		let keys = tempfile::tempdir().unwrap();
		let file = keys.path().join("kp.json");
		std::fs::write(&file, "[1,2,3]").unwrap();
		let error = KeypairFile::resolve(root.path(), &Secret::new(file.display().to_string()))
			.err()
			.unwrap()
			.to_string();
		assert!(error.contains("doesn't hold a Solana keypair"), "{error}");
	}

	#[test]
	fn a_keypair_inside_the_project_must_be_ignored_by_git() {
		let project = tempfile::tempdir().unwrap();
		let file = project.path().join("publish-keypair.json");
		std::fs::write(&file, keypair_json()).unwrap();
		let error = KeypairFile::resolve(project.path(), &Secret::new(file.display().to_string()))
			.err()
			.unwrap()
			.to_string();
		assert!(error.contains("git doesn't ignore"), "{error}");
	}

	#[test]
	fn the_balance_comes_from_one_rpc_call() {
		let (base, seen) = store(Box::new(|_, _| {
			json(serde_json::json!({
				"jsonrpc": "2.0", "id": 1,
				"result": { "context": { "slot": 1 }, "value": 20_000_000 },
			}))
		}));
		let http = Http::default();
		assert_eq!(balance(&http, &base, "Addr").unwrap(), 20_000_000);
		assert_eq!(seen.lock().unwrap().len(), 1);
	}

	#[test]
	fn an_rpc_that_names_no_balance_is_an_error() {
		let (base, _server) = store(Box::new(|_, _| {
			json(serde_json::json!({ "jsonrpc": "2.0" }))
		}));
		let error = balance(&Http::default(), &base, "Addr")
			.unwrap_err()
			.to_string();
		assert!(error.contains("without a balance"), "{error}");
	}

	/// Reaches an address nothing listens on.
	fn closed_port() -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		drop(listener);
		format!("http://{address}")
	}

	#[test]
	fn an_unreachable_rpc_is_an_error_the_run_warns_about() {
		let error = balance(&Http::default(), &closed_port(), "Addr")
			.err()
			.unwrap()
			.to_string();
		assert!(error.contains("could not reach"), "{error}");
	}
}
