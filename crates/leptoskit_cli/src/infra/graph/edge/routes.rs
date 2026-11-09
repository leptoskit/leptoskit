use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::graph::plan::Plan;
use leptoskit_config::graph::routes::Route;
use serde_json::Value;
use serde_json::json;

use super::Bindings;
use super::Target;

// The decoded path must remain canonical, and the original path must not
// contain an encoded separator. Query strings are intentionally excluded.
fn invalid_path() -> Result<String> {
	let decoded = serde_json::to_string(r"(^|/)\.{1,2}(/|$)|//|[\\\x00-\x1f\x7f]")?;
	let escaped = serde_json::to_string(r"^[^?]*%(?i:2f|5c)")?;
	Ok(format!(
		"!{{http.request.uri.path}}.startsWith('/') || \
		 {{http.request.uri.path}}.matches({decoded}) || \
		 {{http.request.orig_uri}}.matches({escaped})"
	))
}

pub(super) fn render(plan: &Plan, host: &str, bindings: &Bindings) -> Result<Vec<Value>> {
	let mut owners: Vec<_> = plan
		.routes()
		.entries()
		.iter()
		.filter(|route| route.host == host)
		.collect();
	ensure!(!owners.is_empty(), "edge host has no public routes");
	ensure!(
		owners.len() == bindings.targets.len(),
		"edge bindings differ from owned routes"
	);
	owners.sort_by(|a, b| {
		b.mount
			.len()
			.cmp(&a.mount.len())
			.then_with(|| a.domain.cmp(&b.domain))
	});
	let mut routes = vec![json!({
		"match":[{"expression":invalid_path()?}],
		"handle":[{"handler":"static_response","status_code":400}],"terminal":true
	})];
	for route in owners {
		ensure!(
			!route.mount.contains(['{', '}']),
			"edge mounts cannot contain placeholder braces"
		);
		let target = bindings
			.targets
			.get(&route.unit)
			.context("public unit has no edge binding")?;
		if route.mount != "/" {
			routes.push(redirect(route)?);
		}
		let expression = format!(
			"{{http.request.uri.path}}.startsWith({})",
			serde_json::to_string(&route.mount)?
		);
		routes.push(json!({
			"match":[{"host":[route.domain,format!("{}.",route.domain)],"expression":expression}],
			"handle":[{"handler":"subroute","routes":handlers(route,target)?}],"terminal":true
		}));
	}
	routes
		.push(json!({"handle":[{"handler":"static_response","status_code":404}],"terminal":true}));
	Ok(routes)
}

fn redirect(route: &Route) -> Result<Value> {
	let bare = route.mount.trim_end_matches('/');
	let expression = format!(
		"{{http.request.uri.path}} == {}",
		serde_json::to_string(bare)?
	);
	Ok(
		json!({"match":[{"host":[route.domain,format!("{}.",route.domain)],"expression":expression}],
			"handle":[{"handler":"static_response", "status_code":308,
				"headers":{"Location":[format!("{}{{http.request.uri.prefixed_query}}",route.mount)]}}],"terminal":true
		}),
	)
}

fn handlers(route: &Route, target: &Target) -> Result<Vec<Value>> {
	match target {
		Target::Static(root) => {
			let mut handle = vec![
				json!({"handler":"headers", "response":{"set":{"X-Content-Type-Options":["nosniff"]}}}),
			];
			if route.mount != "/" {
				handle.push(
					json!({"handler":"rewrite", "strip_path_prefix":route.mount.trim_end_matches('/')}),
				);
			}
			handle.push(json!({"handler":"file_server", "root":super::literal_path(root)?,"index_names":["index.html"]}));
			Ok(vec![
				json!({"match":[{"not":[{"method":["GET","HEAD"]}]}],"handle":[{"handler":"static_response","status_code":405,"headers":{"Allow":["GET, HEAD"]}}],"terminal":true}),
				json!({"handle":handle,"terminal":true}),
			])
		}
		Target::Proxy { port, isolated } => {
			ensure!(*port > 0, "edge upstream port must be nonzero");
			let mut removed = vec![
				"Forwarded",
				"X-Forwarded-*",
				"X-Real-IP",
				"CF-Connecting-IP",
			];
			if *isolated {
				removed.push("Cookie");
			}
			let mut proxy = json!({"handler":"reverse_proxy",
				"upstreams":[{"dial":format!("127.0.0.1:{port}")}],"flush_interval":-1,
				"transport":{"protocol":"http","dial_timeout":"5s","response_header_timeout":"30s"},
				"headers":{"request":{"set":{
					"Host":[route.domain],"X-Forwarded-Host":[route.domain],
					"X-Forwarded-Proto":["https"],"X-Forwarded-For":["{http.vars.client_ip}"]
				}}}
			});
			if *isolated {
				proxy["headers"]["response"] = json!({"delete":["Set-Cookie"]});
			}
			// Strip caller-supplied proxy headers before reverse_proxy's own
			// forwarding logic; install only the edge-derived values afterward.
			Ok(vec![
				json!({"handle":[{"handler":"headers","request":{"delete":removed}},proxy],"terminal":true}),
			])
		}
	}
}
