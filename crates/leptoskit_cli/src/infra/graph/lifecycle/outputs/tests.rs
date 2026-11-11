use super::*;

fn values() -> (LeptoskitConfig, Value, Value) {
	let config = super::super::tests::config();
	let hosts = json!({"web":{"provider":"hetzner","location":"hel1","server_type":"cx23","ipv4":"192.0.2.10","ipv6":"2001:db8::10"},
		"compiler":{"provider":"hetzner","location":"hel1","server_type":"cpx32","ipv4":"192.0.2.11","ipv6":"2001:db8::11"}});
	let domains = json!({"web":["leptoskit.dev"],"compiler":["play-api.leptoskit.dev"]});
	(config, hosts, domains)
}

fn check(config: &LeptoskitConfig, hosts: &Value, domains: &Value) -> Result<Value> {
	checked(
		config,
		&serde_json::to_vec(hosts)?,
		&serde_json::to_vec(domains)?,
	)
}

#[test]
fn only_public_bindings_are_returned_with_observation_limits() {
	let (config, hosts, domains) = values();
	let result = check(&config, &hosts, &domains).unwrap();
	assert_eq!(result["hosts"], hosts);
	assert_eq!(result["domains_by_host"], domains);
	assert_eq!(result["evidence"]["application_runtime"], "not_checked");
	assert_eq!(result["evidence"]["runtime_attestation"], "absent");
}

#[test]
fn changed_host_placement_addresses_and_domains_are_rejected() {
	for (key, value) in [
		("provider", "digitalocean"),
		("location", "fsn1"),
		("server_type", "cpx42"),
		("ipv4", "127.0.0.1"),
		("ipv4", "10.0.0.1"),
		("ipv4", "169.254.0.1"),
		("ipv4", "not-an-address"),
		("ipv6", "::1"),
		("ipv6", "fc00::1"),
		("ipv6", "fe80::1"),
		("ipv6", "::ffff:192.0.2.10"),
	] {
		let (config, mut hosts, domains) = values();
		hosts["web"][key] = json!(value);
		assert!(check(&config, &hosts, &domains).is_err(), "{key}={value}");
	}
	let (config, mut hosts, mut domains) = values();
	hosts.as_object_mut().unwrap().remove("compiler");
	assert!(check(&config, &hosts, &domains).is_err());
	let (_, hosts, _) = values();
	domains["web"] = json!(["other.example.com"]);
	assert!(check(&config, &hosts, &domains).is_err());
}

#[test]
fn extra_sensitive_output_fields_are_rejected_without_printing_their_values() {
	let (config, mut hosts, domains) = values();
	hosts["web"]["tunnel_token"] = json!("fixture-secret-do-not-print");
	let error = check(&config, &hosts, &domains).unwrap_err();
	assert!(!format!("{error:#}").contains("fixture-secret-do-not-print"));
	hosts["web"].as_object_mut().unwrap().remove("tunnel_token");
	hosts["web"]["provider"] = json!("fixture-secret-do-not-print");
	let error = check(&config, &hosts, &domains).unwrap_err();
	assert!(!format!("{error:#}").contains("fixture-secret-do-not-print"));
}
