//! Explicit opt-in acceptance against an owned disposable Linux host.

use std::fmt::Write as _;

use super::*;

fn remote(host: &str, command: &str) -> Vec<u8> {
	remote::execute(host, command, None).unwrap()
}

// Fedora labels are disabled only for this owned fixture container; the
// production managed-host preset uses Docker on Ubuntu.
fn boot(host: &str, image: &str, configuration: &str) {
	let config: Value = serde_json::from_str(configuration).unwrap();
	let mut command = String::from(
		"docker rm -f a05-window-web-v1 >/dev/null 2>&1 || true; docker run -di --name \
		 a05-window-web-v1 --security-opt label=disable --network host --env \
		 DATABASE_URL=postgres://postgres@127.0.0.1:54320/a05?sslmode=disable",
	);

	for (key, value) in config["env"]["clear"].as_object().unwrap() {
		write!(
			command,
			" --env {}",
			remote::quote(&format!("{key}={}", value.as_str().unwrap()))
		)
		.unwrap();
	}

	if let Some(volumes) = config["volumes"].as_array() {
		for volume in volumes {
			write!(command, " -v {}", remote::quote(volume.as_str().unwrap())).unwrap();
		}
	}

	write!(command, " {}", remote::quote(image)).unwrap();

	remote(host, &command);
	remote(
		host,
		"for attempt in $(seq 1 50); do docker logs a05-window-web-v1 2>&1 | grep -q \
		 '^ready:old$' && exit 0; sleep .1; done; exit 1",
	);
}

#[test]
#[ignore = "requires an explicitly owned disposable Linux host, real engine and PostgreSQL"]
fn real_managed_window_delivery_and_recovery() {
	let host = std::env::var("LEPTOSKIT_WINDOW_ACCEPTANCE_HOST").unwrap();
	assert_eq!(host, "a05-owned");
	let directory = std::env::var("LEPTOSKIT_WINDOW_ACCEPTANCE_EVIDENCE").unwrap();
	let directory = Path::new(&directory);
	let descriptor_path = directory.join("descriptor.json");
	let bytes = fs::read(&descriptor_path).unwrap();
	let descriptor =
		super::super::verified::read(&descriptor_path, &super::super::digest(&bytes)).unwrap();

	let authorization: super::super::approvals::Evidence =
		serde_json::from_slice(&fs::read(directory.join("backend-authorization.json")).unwrap())
			.unwrap();
	super::super::approvals::authorize(
		directory,
		"a05.invalid",
		Some(&descriptor),
		Some(&authorization),
	)
	.unwrap();
	let window = descriptor.migration_window().unwrap().unwrap();
	let image = descriptor.engine_image_id.clone();
	descriptor
		.check_remote(&host, &image, &descriptor.migrations)
		.unwrap();
	let hosts = vec![host.clone()];
	let config: ManagedInfraConfig = serde_json::from_value(json!({"name":"a05-window", "domain":"a05.invalid", "location":"local", "server_type":"local", "ssh_key_ids":[], "ssh_cidrs":[], "account_id":"local", "zone_id":"local"})).unwrap();
	let reservation = Reservation::acquire(&host, &config.name).unwrap();
	let original = serde_json::to_string_pretty(
		&json!({"service":"a05-window", "env":{"clear":{}, "secret":["DATABASE_URL"]}}),
	)
	.unwrap()
		+ "\n";
	fs::write(directory.join("release.yml"), &original).unwrap();
	remote(
		&host,
		&format!(
			"docker run --rm --network host --env \
			 DATABASE_URL=postgres://postgres@127.0.0.1:54320/a05?sslmode=disable {} --bootstrap",
			remote::quote(&image)
		),
	);
	prepare(directory, &config, &hosts, Some(&window), &reservation).unwrap();
	let configured = fs::read_to_string(directory.join("release.yml")).unwrap();
	boot(&host, &image, &configured);
	window.verify_running(&hosts, &config.name, "v1").unwrap();
	fs::write(
		directory.join("running-window.json"),
		remote(&host, "docker inspect a05-window-web-v1"),
	)
	.unwrap();
	// A valid replacement still needs the original independently approved pin.
	let file = remote::quote(&window.host_path(&config.name).unwrap());
	remote(
		&host,
		&format!("chmod 644 {file}; printf '\n' >> {file}; chmod 444 {file}"),
	);
	assert!(window.verify_running(&hosts, &config.name, "v1").is_err());
	assert!(prepare(directory, &config, &hosts, Some(&window), &reservation).is_err());
	remote(&host, &format!("rm {file}"));
	prepare(directory, &config, &hosts, Some(&window), &reservation).unwrap();
	// Missing startup fields and a writable bind are independently refused.
	boot(&host, &image, &original);
	assert!(window.verify_running(&hosts, &config.name, "v1").is_err());
	let writable = configured.replace(":ro\"", ":rw\"");
	boot(&host, &image, &writable);
	assert!(window.verify_running(&hosts, &config.name, "v1").is_err());
	let candidate_json = format!("{}\n", window.json);
	let candidate = Window::new(&candidate_json, &leptoskit_db::checksum(&candidate_json)).unwrap();
	prepare(directory, &config, &hosts, Some(&candidate), &reservation).unwrap();
	boot(
		&host,
		&image,
		&fs::read_to_string(directory.join("release.yml")).unwrap(),
	);
	candidate
		.verify_running(&hosts, &config.name, "v1")
		.unwrap();
	prepare(directory, &config, &hosts, Some(&window), &reservation).unwrap();
	boot(
		&host,
		&image,
		&fs::read_to_string(directory.join("release.yml")).unwrap(),
	);
	window.verify_running(&hosts, &config.name, "v1").unwrap();
	prepare(directory, &config, &hosts, None, &reservation).unwrap();
	assert_eq!(
		fs::read_to_string(directory.join("release.yml")).unwrap(),
		original
	);
	boot(&host, &image, &original);
	fs::write(
		directory.join("recovered-no-window.json"),
		remote(&host, "docker inspect a05-window-web-v1"),
	)
	.unwrap();
	let lock = remote::quote(&format!("{}/operation.lock", remote::base(&config.name)));
	remote(&host, &format!("printf lost > {lock}"));
	assert!(prepare(directory, &config, &hosts, Some(&window), &reservation).is_err());
	remote(
		&host,
		&format!("printf %s {} > {lock}", remote::quote(&reservation.owner)),
	);
	remote(&host, "docker rm -f a05-window-web-v1");
}
