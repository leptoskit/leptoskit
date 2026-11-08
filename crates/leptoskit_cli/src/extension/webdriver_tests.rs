use super::*;

/// Runs without an app build, so startup/install/reinstall can be checked
/// independently from the Rust-to-wasm toolchain.
#[test]
#[ignore = "needs Firefox and geckodriver; set LEPTOSKIT_FIREFOX and LEPTOSKIT_GECKODRIVER"]
fn firefox_installs_reloads_and_stops() {
	let extension = tempfile::tempdir().unwrap();
	std::fs::write(
		extension.path().join("manifest.json"),
		json!({
			"manifest_version": 3, "name": "leptoskit smoke", "version": "1.0",
			"browser_specific_settings": {"gecko": {"id": "smoke@leptoskit.test"}},
			"background": {"scripts": ["background.js"]}
		})
		.to_string(),
	)
	.unwrap();
	std::fs::write(
		extension.path().join("background.js"),
		"globalThis.__leptoskit_background_ready = true;",
	)
	.unwrap();
	std::fs::write(
		extension.path().join("probe.html"),
		"<!doctype html><title>Installed</title><p>Extension installed</p>",
	)
	.unwrap();
	let profile = tempfile::tempdir().unwrap();
	let mut driver = Driver::launch(
		ExtensionBrowser::Firefox,
		extension.path(),
		profile.path(),
		true,
	)
	.unwrap();
	driver
		.open(&format!("{}/probe.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Installed'", Duration::from_secs(5))
		.unwrap();

	std::fs::write(
		extension.path().join("probe.html"),
		"<!doctype html><title>Reloaded</title>",
	)
	.unwrap();
	driver.reinstall(extension.path()).unwrap();
	driver
		.open(&format!("{}/probe.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Reloaded'", Duration::from_secs(5))
		.unwrap();
}

/// Fake geckodriver: validates negotiation and lifetime without requiring a
/// graphical environment or a particular Firefox installation.
#[test]
fn firefox_session_installs_retries_and_tears_down() {
	use std::net::TcpListener;
	let http = TcpListener::bind("127.0.0.1:0").unwrap();
	let ws = TcpListener::bind("127.0.0.1:0").unwrap();
	let base = format!("http://{}", http.local_addr().unwrap());
	let socket = format!("ws://{}/session", ws.local_addr().unwrap());
	let extension = tempfile::tempdir().unwrap();
	let path = extension.path().to_path_buf();
	let web_socket = std::thread::spawn(move || fake_extension_socket(&ws, &path));
	let requests = std::thread::spawn(move || fake_webdriver(&http, &socket));

	let mut driver = Driver {
		process: None,
		isolated: false,
		base,
		session: String::new(),
		extension_base: String::new(),
		bidi: None,
		addon: None,
		agent: ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_secs(3)))
			.build()
			.into(),
	};
	driver
		.start_firefox(
			extension.path(),
			Path::new("/fake/firefox"),
			"test@example.com",
			"test-uuid",
			true,
		)
		.unwrap();
	assert_eq!(driver.extension_base, "moz-extension://test-uuid");
	driver.open("moz-extension://test-uuid/probe.html").unwrap();
	assert_eq!(driver.script("return true").unwrap(), true);
	let error = driver.reinstall(extension.path()).unwrap_err().to_string();
	assert!(error.contains("invalid manifest"), "{error}");
	// After uninstall succeeds and installation fails, the next rebuild can
	// retry installation rather than trying to uninstall a missing add-on.
	driver.reinstall(extension.path()).unwrap();
	drop(driver);
	requests.join().unwrap();
	web_socket.join().unwrap();
}

fn fake_extension_socket(listener: &std::net::TcpListener, path: &Path) {
	let (stream, _) = listener.accept().unwrap();
	stream
		.set_read_timeout(Some(Duration::from_secs(5)))
		.unwrap();
	let mut socket = tungstenite::accept(stream).unwrap();

	for (index, method) in [
		"webExtension.install",
		"webExtension.uninstall",
		"webExtension.install",
		"webExtension.install",
	]
	.iter()
	.enumerate()
	{
		let message = socket.read().unwrap();
		let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
		assert_eq!(request["method"], *method);
		let params = if *method == "webExtension.uninstall" {
			json!({"extension": "test@example.com"})
		} else {
			json!({"extensionData": {"type": "path", "path": path}})
		};

		assert_eq!(request["params"], params);
		let response = if index == 2 {
			json!({"id":request["id"], "type":"error", "error":"invalid web extension", "message":"invalid manifest"})
		} else {
			json!({"id":request["id"], "type":"success", "result":{"extension":"test@example.com"}})
		};

		socket
			.send(tungstenite::Message::Text(response.to_string().into()))
			.unwrap();
	}
}

fn fake_webdriver(listener: &std::net::TcpListener, socket: &str) {
	use std::io::Write as _;

	for expected in [
		"POST /session HTTP/1.1",
		"POST /session/test/url HTTP/1.1",
		"POST /session/test/execute/async HTTP/1.1",
		"DELETE /session/test HTTP/1.1",
	] {
		let (mut stream, _) = listener.accept().unwrap();
		stream
			.set_read_timeout(Some(Duration::from_secs(5)))
			.unwrap();
		let (line, request) = read_request(&mut stream);
		assert_eq!(line.trim(), expected);

		let result = match expected {
			"POST /session HTTP/1.1" => {
				let caps = &request["capabilities"]["alwaysMatch"];
				assert_eq!(caps["browserName"], "firefox");
				assert_eq!(caps["webSocketUrl"], true);
				assert_eq!(caps["moz:firefoxOptions"]["args"], json!(["-headless"]));
				assert_eq!(caps["moz:firefoxOptions"]["binary"], "/fake/firefox");
				let uuids: Value = serde_json::from_str(
					caps["moz:firefoxOptions"]["prefs"]["extensions.webextensions.uuids"]
						.as_str()
						.unwrap(),
				)
				.unwrap();
				assert_eq!(uuids, json!({"test@example.com":"test-uuid"}));
				assert_eq!(
					caps["moz:firefoxOptions"]["prefs"]
						["extensions.webextensions.keepUuidOnUninstall"],
					true
				);
				assert_eq!(
					caps["moz:firefoxOptions"]["prefs"]
						["extensions.webextensions.keepStorageOnUninstall"],
					true
				);
				json!({"sessionId":"test", "capabilities":{"webSocketUrl":socket}})
			}

			"POST /session/test/url HTTP/1.1" => {
				assert_eq!(request["url"], "moz-extension://test-uuid/probe.html");
				Value::Null
			}

			"POST /session/test/execute/async HTTP/1.1" => {
				assert_eq!(request["script"], "return true");
				json!(true)
			}

			_ => Value::Null,
		};

		let body = json!({"value": result}).to_string();
		write!(
			stream,
			"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
			 {}\r\nconnection: close\r\n\r\n{body}",
			body.len()
		)
		.unwrap();
	}
}

// Read exactly one HTTP request; the fake server closes each connection
// after replying so its request order and session teardown are observable.
fn read_request(stream: &mut std::net::TcpStream) -> (String, Value) {
	use std::io::BufRead as _;
	use std::io::Read as _;
	let mut reader = std::io::BufReader::new(stream);
	let mut first = String::new();
	reader.read_line(&mut first).unwrap();
	let mut length = 0;

	loop {
		let mut line = String::new();
		reader.read_line(&mut line).unwrap();

		if line == "\r\n" {
			break;
		}

		if let Some((name, value)) = line.split_once(':')
			&& name.eq_ignore_ascii_case("content-length")
		{
			length = value.trim().parse().unwrap();
		}
	}

	let mut body = vec![0; length];
	reader.read_exact(&mut body).unwrap();
	(
		first,
		if body.is_empty() {
			Value::Null
		} else {
			serde_json::from_slice(&body).unwrap()
		},
	)
}

fn driver(base: String, session: &str) -> Driver {
	Driver {
		process: None,
		isolated: false,
		base,
		session: session.to_owned(),
		extension_base: String::new(),
		bidi: None,
		addon: None,
		agent: ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_secs(3)))
			.http_status_as_error(false)
			.build()
			.into(),
	}
}

/// A loopback HTTP server that answers each connection in order with `values`
/// wrapped as a `WebDriver` result, then stops.
fn scripted_driver(values: Vec<Value>) -> (String, std::thread::JoinHandle<()>) {
	use std::io::Write as _;

	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let thread = std::thread::spawn(move || {
		for value in values {
			let Ok((mut stream, _)) = listener.accept() else {
				return;
			};
			stream
				.set_read_timeout(Some(Duration::from_secs(5)))
				.unwrap();
			let _ = read_request(&mut stream);
			let body = json!({"value": value}).to_string();
			write!(
				stream,
				"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
				 {}\r\nconnection: close\r\n\r\n{body}",
				body.len()
			)
			.unwrap();
		}
	});
	(base, thread)
}

#[test]
fn call_surfaces_webdriver_errors_and_uses_each_method() {
	let (base, server) = scripted_driver(vec![
		json!({"ready": true}),
		json!({"error": "script timeout", "message": "the page never finished"}),
		Value::Null,
	]);
	let driver = driver(base, "error-session");

	assert_eq!(
		driver.call("GET", "/status", None).unwrap(),
		json!({"ready": true})
	);
	let error = driver.script("boom").unwrap_err().to_string();
	assert_eq!(error, "script timeout: the page never finished");
	assert_eq!(
		driver
			.call("DELETE", "/session/error-session", None)
			.unwrap(),
		Value::Null
	);
	server.join().unwrap();
}

#[test]
fn call_surfaces_an_error_without_a_message() {
	let (base, server) = scripted_driver(vec![json!({"error": "no such session"})]);
	let driver = driver(base, "gone");

	let error = driver.call("POST", "/session/gone/url", None).unwrap_err();
	assert_eq!(error.to_string(), "no such session: ");
	server.join().unwrap();
}

#[test]
fn wait_reports_the_condition_it_gave_up_on() {
	use std::net::TcpListener;
	use std::sync::Arc;
	use std::sync::atomic::AtomicBool;
	use std::sync::atomic::Ordering;
	use std::time::Instant;

	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let stop = Arc::new(AtomicBool::new(false));
	let running = Arc::clone(&stop);
	let server = std::thread::spawn(move || {
		let deadline = Instant::now() + Duration::from_secs(5);
		while !running.load(Ordering::SeqCst) && Instant::now() < deadline {
			match listener.accept() {
				Ok((mut stream, _)) => {
					// Accepted sockets inherit nonblocking mode on macOS.
					let _ = stream.set_nonblocking(false);
					let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
					let _ = read_request(&mut stream);
					let body = json!({"value": false}).to_string();
					use std::io::Write as _;
					let _ = write!(
						stream,
						"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
						 {}\r\nconnection: close\r\n\r\n{body}",
						body.len()
					);
				}
				Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
					std::thread::sleep(Duration::from_millis(2));
				}

				Err(_) => break,
			}
		}
	});
	let driver = driver(base, "waiting");

	let error = driver
		.wait("document.title === 'ready'", Duration::from_millis(50))
		.unwrap_err()
		.to_string();

	assert!(
		error.contains("timed out waiting for document.title === 'ready'"),
		"{error}"
	);
	stop.store(true, Ordering::SeqCst);
	server.join().unwrap();
}

#[test]
fn a_missing_extension_package_is_refused_before_a_driver_starts() {
	let missing = tempfile::tempdir().unwrap().path().join("missing");
	let profile = tempfile::tempdir().unwrap();
	let Err(error) = Driver::launch(ExtensionBrowser::Chromium, &missing, profile.path(), true)
	else {
		panic!("a missing extension package must not start a driver");
	};
	let error = error.to_string();

	assert!(
		error.contains("extension package directory is missing"),
		"{error}"
	);
}

#[cfg(unix)]
fn executable(path: &Path, body: &str) {
	use std::os::unix::fs::PermissionsExt as _;

	std::fs::write(path, body).unwrap();
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The stub driver publishes the port it was told to use, then waits to be
/// killed. The session tests serve `WebDriver` on that port.
#[cfg(unix)]
fn stub_driver(directory: &Path) -> (PathBuf, PathBuf) {
	let port_file = directory.join("port");
	let script = directory.join("fake-chromedriver");
	executable(
		&script,
		&format!(
			"#!/bin/sh\nfor argument in \"$@\"; do\ncase \"$argument\" in\n--port=*) printf '%s' \
			 \"${{argument#--port=}}\" > '{}';;\nesac\ndone\nexec sleep 30\n",
			port_file.display()
		),
	);
	(script, port_file)
}

/// Waits for a stub driver's port and answers its `WebDriver` requests,
/// recording each one, until the session is deleted.
#[cfg(unix)]
fn fake_chromedriver(port_file: &Path, requests: &std::sync::Mutex<Vec<(String, String, Value)>>) {
	use std::io::Write as _;
	use std::time::Instant;

	let deadline = Instant::now() + Duration::from_secs(10);

	let port = loop {
		if let Ok(text) = std::fs::read_to_string(port_file)
			&& let Ok(port) = text.trim().parse::<u16>()
		{
			break port;
		}

		assert!(
			Instant::now() < deadline,
			"the stub driver never published its port"
		);
		std::thread::sleep(Duration::from_millis(10));
	};

	let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();

	loop {
		let (mut stream, _) = listener.accept().unwrap();
		stream
			.set_read_timeout(Some(Duration::from_secs(5)))
			.unwrap();
		let (line, body) = read_request(&mut stream);
		let mut parts = line.split_whitespace();
		let method = parts.next().unwrap_or_default().to_owned();
		let path = parts.next().unwrap_or_default().to_owned();
		requests
			.lock()
			.unwrap()
			.push((method.clone(), path.clone(), body.clone()));
		let value = if method == "GET" {
			json!({"ready": true})
		} else if method == "POST" && path == "/session" {
			json!({"sessionId": "chrome-session", "capabilities": {}})
		} else if path.ends_with("/execute/async") {
			let script = body["script"].as_str().unwrap_or_default();

			if script.contains("document.title") {
				json!(true)
			} else {
				json!(2)
			}
		} else {
			Value::Null
		};

		let body = json!({"value": value}).to_string();
		write!(
			stream,
			"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
			 {}\r\nconnection: close\r\n\r\n{body}",
			body.len()
		)
		.unwrap();

		if method == "DELETE" {
			return;
		}
	}
}

#[test]
fn a_missing_driver_executable_names_leptoskit_doctor() {
	let Err(error) = Driver::spawn(Path::new("/nonexistent/chromedriver"), false) else {
		panic!("a missing driver executable must not start");
	};
	let error = error.to_string();

	assert!(
		error.contains("could not start /nonexistent/chromedriver"),
		"{error}"
	);
	assert!(error.contains("leptoskit doctor"), "{error}");
}

#[test]
fn the_installer_cache_is_searched_for_chromedriver() {
	let install = tempfile::tempdir().unwrap();
	assert_eq!(chromedriver_in(install.path()), None);

	let executable = if cfg!(windows) {
		"chromedriver.exe"
	} else {
		"chromedriver"
	};

	let linux = install.path().join("chromedriver-linux64");
	std::fs::create_dir_all(&linux).unwrap();
	std::fs::write(linux.join(executable), "").unwrap();
	assert_eq!(
		chromedriver_in(install.path()),
		Some(linux.join(executable))
	);

	// The macOS arm64 install is preferred over the other layouts.
	let mac = install.path().join("chromedriver-mac-arm64");
	std::fs::create_dir_all(&mac).unwrap();
	std::fs::write(mac.join(executable), "").unwrap();
	assert_eq!(chromedriver_in(install.path()), Some(mac.join(executable)));
}

#[test]
fn chromedriver_resolution_prefers_the_environment_and_finds_the_cache() {
	let root = tempfile::tempdir().unwrap();
	let override_path = root.path().join("override-chromedriver");
	std::fs::write(&override_path, "#!/bin/sh\nexit 0\n").unwrap();
	let executable = if cfg!(windows) {
		"chromedriver.exe"
	} else {
		"chromedriver"
	};

	let install = root.path().join(".cache/puppeteer/chromedriver");

	for version in ["1.0", "2.0"] {
		let directory = install.join(version).join("chromedriver-linux64");

		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(directory.join(executable), "").unwrap();
	}

	let output = Command::new(std::env::current_exe().unwrap())
		.args([
			"--ignored",
			"--exact",
			"extension::webdriver::tests::chromedriver_resolution_fixture",
			"--test-threads=1",
		])
		.env("HOME", root.path())
		.env("USERPROFILE", root.path())
		.env(CHROMEDRIVER_ENV, &override_path)
		.env(
			"LEPTOSKIT_TEST_CACHED_CHROMEDRIVER",
			install
				.join("2.0")
				.join("chromedriver-linux64")
				.join(executable),
		)
		.output()
		.unwrap();

	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

#[test]
#[ignore = "subprocess fixture: \
            chromedriver_resolution_prefers_the_environment_and_finds_the_cache runs it"]
fn chromedriver_resolution_fixture() {
	let expected = PathBuf::from(std::env::var_os("LEPTOSKIT_TEST_CACHED_CHROMEDRIVER").unwrap());
	assert_eq!(cached_chromedriver(), Some(expected));
	assert_eq!(
		chromedriver(),
		PathBuf::from(std::env::var_os(CHROMEDRIVER_ENV).unwrap())
	);
}

#[test]
fn chromedriver_falls_back_to_the_path_when_the_cache_is_empty() {
	let root = tempfile::tempdir().unwrap();
	let output = Command::new(std::env::current_exe().unwrap())
		.args([
			"--ignored",
			"--exact",
			"extension::webdriver::tests::chromedriver_path_fixture",
			"--test-threads=1",
		])
		.env("HOME", root.path())
		.env("USERPROFILE", root.path())
		.env_remove(CHROMEDRIVER_ENV)
		.output()
		.unwrap();

	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

#[test]
#[ignore = "subprocess fixture: chromedriver_falls_back_to_the_path_when_the_cache_is_empty runs it"]
fn chromedriver_path_fixture() {
	assert_eq!(cached_chromedriver(), None);
	assert_eq!(chromedriver(), PathBuf::from("chromedriver"));
}

#[cfg(unix)]
#[test]
fn a_chromium_session_is_negotiated_through_the_stub_driver() {
	let root = tempfile::tempdir().unwrap();
	let extension = root.path().join("extension");
	std::fs::create_dir_all(&extension).unwrap();
	std::fs::write(
		extension.join("manifest.json"),
		json!({
			"manifest_version": 3, "name": "leptoskit smoke", "version": "1.0",
			"background": {"service_worker": "background.js"}
		})
		.to_string(),
	)
	.unwrap();
	std::fs::write(
		extension.join("background.js"),
		"chrome.runtime.onMessage.addListener(() => {});",
	)
	.unwrap();
	let profile = root.path().join("profile");
	std::fs::create_dir_all(&profile).unwrap();
	let chrome = root.path().join("chrome");

	executable(&chrome, "#!/bin/sh\nexit 0\n");
	let (script, port_file) = stub_driver(root.path());

	let output = Command::new(std::env::current_exe().unwrap())
		.args([
			"--ignored",
			"--exact",
			"extension::webdriver::tests::chromium_session_fixture",
			"--test-threads=1",
		])
		.env(CHROMEDRIVER_ENV, &script)
		.env(super::super::dev::CHROME_ENV, &chrome)
		.env("LEPTOSKIT_TEST_EXTENSION", &extension)
		.env("LEPTOSKIT_TEST_PROFILE", &profile)
		.env("LEPTOSKIT_TEST_CHROME", &chrome)
		.env("LEPTOSKIT_TEST_PORT_FILE", &port_file)
		.output()
		.unwrap();

	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

#[cfg(unix)]
#[test]
#[ignore = "subprocess fixture: a_chromium_session_is_negotiated_through_the_stub_driver runs it"]
fn chromium_session_fixture() {
	use std::sync::Arc;
	use std::sync::Mutex;

	let extension = PathBuf::from(std::env::var_os("LEPTOSKIT_TEST_EXTENSION").unwrap());
	let profile = PathBuf::from(std::env::var_os("LEPTOSKIT_TEST_PROFILE").unwrap());
	let chrome = PathBuf::from(std::env::var_os("LEPTOSKIT_TEST_CHROME").unwrap());
	let port_file = PathBuf::from(std::env::var_os("LEPTOSKIT_TEST_PORT_FILE").unwrap());
	let requests = Arc::new(Mutex::new(Vec::new()));
	let server = {
		let requests = Arc::clone(&requests);
		let port_file = port_file.clone();
		std::thread::spawn(move || fake_chromedriver(&port_file, &requests))
	};
	let driver = Driver::launch(ExtensionBrowser::Chromium, &extension, &profile, true).unwrap();
	let canonical = extension.canonicalize().unwrap();

	assert_eq!(
		driver.extension_base,
		format!(
			"chrome-extension://{}",
			super::super::dev::unpacked_id(&canonical)
		)
	);
	driver
		.open(&format!("{}/popup.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Ready'", Duration::from_secs(3))
		.unwrap();
	assert_eq!(driver.script("return 1 + 1").unwrap(), json!(2));
	drop(driver);
	server.join().unwrap();

	let requests = requests.lock().unwrap();
	let session = requests
		.iter()
		.find(|(method, path, _)| method == "POST" && path == "/session")
		.expect("the driver must start a session");
	let capabilities = &session.2["capabilities"]["alwaysMatch"];
	assert_eq!(capabilities["browserName"], "chrome");
	assert_eq!(capabilities["timeouts"]["script"], 10000);
	assert_eq!(capabilities["timeouts"]["pageLoad"], 30000);
	assert_eq!(
		capabilities["goog:chromeOptions"]["binary"],
		chrome.display().to_string()
	);
	let args = capabilities["goog:chromeOptions"]["args"]
		.as_array()
		.unwrap()
		.iter()
		.map(|value| value.as_str().unwrap().to_owned())
		.collect::<Vec<_>>();

	for expected in [
		format!("--load-extension={}", canonical.display()),
		format!("--disable-extensions-except={}", canonical.display()),
		format!("--user-data-dir={}", profile.display()),
		"--headless=new".to_owned(),
		"--no-first-run".to_owned(),
		"--no-default-browser-check".to_owned(),
	] {
		assert!(args.contains(&expected), "missing {expected}: {args:?}");
	}
	assert_eq!(
		requests
			.iter()
			.map(|(method, path, _)| format!("{method} {path}"))
			.collect::<Vec<_>>(),
		vec![
			"GET /status".to_owned(),
			"POST /session".to_owned(),
			"POST /session/chrome-session/url".to_owned(),
			"POST /session/chrome-session/execute/async".to_owned(),
			"POST /session/chrome-session/execute/async".to_owned(),
			"DELETE /session/chrome-session".to_owned(),
		]
	);
}

#[cfg(unix)]
#[test]
fn dropping_a_non_isolated_driver_kills_its_process() {
	use std::io::Write as _;
	use std::time::Instant;

	let root = tempfile::tempdir().unwrap();
	let (script, port_file) = stub_driver(root.path());
	let server = std::thread::spawn(move || {
		let deadline = Instant::now() + Duration::from_secs(10);
		let port = loop {
			if let Ok(text) = std::fs::read_to_string(&port_file)
				&& let Ok(port) = text.trim().parse::<u16>()
			{
				break port;
			}

			assert!(Instant::now() < deadline, "the stub driver never started");
			std::thread::sleep(Duration::from_millis(10));
		};

		let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
		let (mut stream, _) = listener.accept().unwrap();
		let _ = read_request(&mut stream);
		let body = json!({"value": {"ready": true}}).to_string();
		write!(
			stream,
			"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
			 {}\r\nconnection: close\r\n\r\n{body}",
			body.len()
		)
		.unwrap();
	});

	let driver = Driver::spawn(&script, false).unwrap();
	assert!(driver.process.is_some());
	assert_eq!(driver.session, "");
	drop(driver);
	server.join().unwrap();
}

/// Keeps the Chromium path checked when sharing the transport with Firefox.
#[test]
#[ignore = "needs Chrome for Testing and chromedriver"]
fn chromium_still_loads_an_unpacked_extension() {
	let extension = tempfile::tempdir().unwrap();
	std::fs::write(
		extension.path().join("manifest.json"),
		json!({
			"manifest_version": 3, "name": "leptoskit smoke", "version": "1.0",
			"background": {"service_worker": "background.js"}
		})
		.to_string(),
	)
	.unwrap();
	std::fs::write(
		extension.path().join("background.js"),
		"chrome.runtime.onMessage.addListener(() => {});",
	)
	.unwrap();
	std::fs::write(
		extension.path().join("probe.html"),
		"<!doctype html><title>Installed</title>",
	)
	.unwrap();
	let profile = tempfile::tempdir().unwrap();
	let driver = Driver::launch(
		ExtensionBrowser::Chromium,
		extension.path(),
		profile.path(),
		true,
	)
	.unwrap();
	driver
		.open(&format!("{}/probe.html", driver.extension_base))
		.unwrap();
	driver
		.wait("document.title === 'Installed'", Duration::from_secs(5))
		.unwrap();
}
