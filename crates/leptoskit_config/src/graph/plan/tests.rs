use super::*;

fn frontend(name: &str) -> Unit {
	Unit::Frontend(name.into())
}
fn service(name: &str) -> Unit {
	Unit::Service(name.into())
}
fn placement(unit: Unit, host: &str) -> Placement {
	Placement {
		unit,
		host: host.into(),
		requires_exclusive_host: false,
		container_runtime: None,
		artifacts: Vec::new(),
		depends_on: Vec::new(),
	}
}
fn hosts() -> Vec<Host> {
	vec![
		Host {
			name: "web".into(),
			exclusive: false,
			container_runtime: None,
		},
		Host {
			name: "compiler".into(),
			exclusive: true,
			container_runtime: Some(ContainerRuntime::RootlessPodman),
		},
	]
}
fn website() -> Plan {
	let mut placements = ["site", "docs", "play"]
		.map(|name| placement(frontend(name), "web"))
		.to_vec();

	for unit in &mut placements {
		unit.artifacts.push("public-site".into());
	}

	let mut compiler = placement(service("play-api"), "compiler");

	compiler.artifacts = vec!["play-api".into(), "toolchain".into()];
	compiler.requires_exclusive_host = true;
	compiler.container_runtime = Some(ContainerRuntime::RootlessPodman);
	placements.push(compiler);
	let mut routes = [("site", "/"), ("docs", "/docs/"), ("play", "/play/")]
		.map(|(name, mount)| {
			Route {
				unit: frontend(name),
				host: "web".into(),
				domain: "leptoskit.dev".into(),
				mount: mount.into(),
			}
		})
		.to_vec();
	routes.push(Route {
		unit: service("play-api"),
		host: "compiler".into(),
		domain: "play-api.leptoskit.dev".into(),
		mount: "/".into(),
	});
	let trees = [("site", "."), ("docs", "docs"), ("play", "play")]
		.map(|(name, directory)| {
			Tree {
				unit: frontend(name),
				artifact: "public-site".into(),
				directory: directory.into(),
			}
		})
		.to_vec();
	Plan::new(
		hosts(),
		vec!["public-site".into(), "play-api".into(), "toolchain".into()],
		placements,
		routes,
		trees,
		vec![Call {
			caller: frontend("play"),
			target: service("play-api"),
		}],
	)
	.unwrap()
}

#[test]
fn website_has_a_separate_compiler_without_a_readiness_dependency_for_a_browser_call() {
	let plan = website();
	assert_eq!(plan.hosts().len(), 2);
	assert_eq!(plan.artifacts().len(), 3);
	assert_eq!(plan.placements().len(), 4);
	assert_eq!(plan.stages().len(), 1);
	assert_eq!(plan.calls().len(), 1);
	assert_eq!(plan.routes().entries().len(), 4);
	assert_eq!(plan.trees().entries().len(), 3);
}

#[test]
fn docs_only_selects_one_release_even_when_build_recipe_is_shared() {
	let plan = website();
	assert_eq!(
		plan.select(&[frontend("docs")]).unwrap(),
		Selection {
			stages: vec![vec![frontend("docs")]],
			readiness: vec![],
			artifacts: vec!["public-site".into()]
		}
	);
	let selected = plan.select(&[frontend("docs"), frontend("site")]).unwrap();
	assert_eq!(selected.artifacts, ["public-site"]);
	assert!(
		!selected
			.stages
			.iter()
			.flatten()
			.any(|unit| unit == &service("play-api"))
	);
}

#[test]
fn compiler_release_selects_backend_and_toolchain_without_rebuilding_the_site() {
	let selected = website().select(&[service("play-api")]).unwrap();
	assert_eq!(selected.stages, [vec![service("play-api")]]);
	assert_eq!(selected.artifacts, ["play-api", "toolchain"]);
	assert_eq!(selected.readiness, Vec::<Unit>::new());
}

#[test]
fn unknown_artifact_inputs_are_not_silently_omitted() {
	let mut api = placement(service("api"), "web");
	api.artifacts.push("missing-build".into());
	assert!(
		Plan::new(hosts(), vec![], vec![api], vec![], vec![], vec![])
			.unwrap_err()
			.contains("undeclared artifact")
	);
}

#[test]
fn a_static_only_project_needs_no_application_service() {
	let mut docs = placement(frontend("docs"), "web");
	docs.artifacts.push("book".into());
	let plan = Plan::new(
		vec![hosts().remove(0)],
		vec!["book".into()],
		vec![docs],
		Vec::new(),
		vec![Tree {
			unit: frontend("docs"),
			artifact: "book".into(),
			directory: ".".into(),
		}],
		Vec::new(),
	)
	.unwrap();
	assert_eq!(plan.stages(), [vec![frontend("docs")]]);
}

#[test]
fn exclusive_compiler_host_rejects_accidental_static_colocation() {
	let plan = Plan::new(
		hosts(),
		vec![],
		vec![
			placement(service("play-api"), "compiler"),
			placement(frontend("docs"), "compiler"),
		],
		vec![],
		vec![],
		vec![],
	);
	assert!(plan.unwrap_err().contains("exclusive host"));
}

#[test]
fn runtime_and_exclusivity_requirements_must_be_declared_by_the_host() {
	let mut compiler = placement(service("play-api"), "web");
	compiler.requires_exclusive_host = true;
	assert!(
		Plan::new(
			hosts(),
			vec![],
			vec![compiler.clone()],
			vec![],
			vec![],
			vec![]
		)
		.unwrap_err()
		.contains("exclusive host")
	);
	compiler.host = "compiler".into();
	compiler.container_runtime = Some(ContainerRuntime::RootlessDocker);
	assert!(
		Plan::new(hosts(), vec![], vec![compiler], vec![], vec![], vec![])
			.unwrap_err()
			.contains("runtime")
	);
}

#[test]
fn readiness_order_is_deterministic_and_nonselected_dependencies_are_checked_not_released() {
	let a = placement(service("a"), "web");
	let mut b = placement(service("b"), "web");
	b.depends_on.push(service("a"));
	let mut c = placement(service("c"), "web");
	c.depends_on.push(service("b"));
	let plan = Plan::new(hosts(), vec![], vec![c, b, a], vec![], vec![], vec![]).unwrap();
	assert_eq!(
		plan.stages(),
		[vec![service("a")], vec![service("b")], vec![service("c")]]
	);
	assert_eq!(
		plan.select(&[service("c")]).unwrap().readiness,
		[service("a"), service("b")]
	);

	assert_eq!(
		plan.select(&[service("c")]).unwrap().stages,
		[vec![service("c")]]
	);
}

#[test]
fn call_cycles_are_allowed_but_readiness_cycles_are_not() {
	let mut a = placement(service("a"), "web");
	let mut b = placement(service("b"), "web");
	let calls = vec![
		Call {
			caller: service("a"),
			target: service("b"),
		},
		Call {
			caller: service("b"),
			target: service("a"),
		},
	];
	assert!(
		Plan::new(
			hosts(),
			vec![],
			vec![a.clone(), b.clone()],
			vec![],
			vec![],
			calls.clone()
		)
		.is_ok()
	);
	a.depends_on.push(service("b"));
	b.depends_on.push(service("a"));
	assert!(
		Plan::new(hosts(), vec![], vec![a, b], vec![], vec![], calls)
			.unwrap_err()
			.contains("cycle")
	);
}

#[test]
fn route_placement_and_artifact_owners_must_match_real_units() {
	let bad_route = Route {
		unit: frontend("site"),
		host: "compiler".into(),
		domain: "leptoskit.dev".into(),
		mount: "/".into(),
	};
	assert!(
		Plan::new(
			hosts(),
			vec![],
			vec![placement(frontend("site"), "web")],
			vec![bad_route],
			vec![],
			vec![]
		)
		.unwrap_err()
		.contains("route uses")
	);
	let unknown = Tree {
		unit: frontend("unknown"),
		artifact: "site".into(),
		directory: ".".into(),
	};
	assert!(
		Plan::new(
			hosts(),
			vec![],
			vec![placement(frontend("site"), "web")],
			vec![],
			vec![unknown],
			vec![]
		)
		.unwrap_err()
		.contains("undeclared")
	);
}

const UNIT_COUNT: usize = 4;
type Reachability = [[bool; UNIT_COUNT]; UNIT_COUNT];

fn graph_case(
	bits: usize,
	units: &[Unit],
	edges: &[(usize, usize)],
) -> (Vec<Placement>, Reachability) {
	let mut placements: Vec<_> = units
		.iter()
		.map(|unit| placement(unit.clone(), "web"))
		.collect();
	let mut reachable = [[false; UNIT_COUNT]; UNIT_COUNT];

	for (bit, &(from, to)) in edges.iter().enumerate() {
		if bits & (1 << bit) != 0 {
			placements[from].depends_on.push(units[to].clone());
			reachable[from][to] = true;
		}
	}

	(placements, transitive_closure(reachable))
}

// Independent Floyd–Warshall reachability oracle. Each pass allows paths
// through one more vertex; this does not repeat ready-stage removal.
fn transitive_closure(mut reachable: Reachability) -> Reachability {
	for through in 0..UNIT_COUNT {
		let onward = reachable[through];

		for row in &mut reachable {
			let arrives = row[through];

			for (to, edge) in row.iter_mut().enumerate() {
				*edge |= arrives && onward[to];
			}
		}
	}

	reachable
}

fn assert_dependency_selections(plan: &Plan, units: &[Unit], reachable: &Reachability) {
	let positions: BTreeMap<_, _> = plan
		.stages()
		.iter()
		.enumerate()
		.flat_map(|(stage, units)| units.iter().map(move |unit| (unit, stage)))
		.collect();
	assert_eq!(positions.len(), UNIT_COUNT);

	for (from, unit) in units.iter().enumerate() {
		let dependencies: Vec<_> = units
			.iter()
			.enumerate()
			.filter(|(to, _)| reachable[from][*to])
			.map(|(_, unit)| unit.clone())
			.collect();
		assert_eq!(
			plan.select(std::slice::from_ref(unit)).unwrap().readiness,
			dependencies
		);

		for dependency in dependencies {
			assert!(positions[&dependency] < positions[unit]);
		}
	}
}

#[test]
fn every_four_unit_readiness_graph_matches_independent_reachability() {
	let units: Vec<_> = (0..UNIT_COUNT)
		.map(|index| service(&format!("s{index}")))
		.collect();
	let edges: Vec<_> = (0..UNIT_COUNT)
		.flat_map(|from| {
			(0..UNIT_COUNT)
				.filter(move |to| *to != from)
				.map(move |to| (from, to))
		})
		.collect();
	let mut acyclic = 0;

	for bits in 0..(1 << edges.len()) {
		let (placements, reachable) = graph_case(bits, &units, &edges);
		let cycle = (0..UNIT_COUNT).any(|index| reachable[index][index]);
		let result = Plan::new(hosts(), vec![], placements, vec![], vec![], vec![]);
		assert_eq!(result.is_err(), cycle, "graph {bits}");

		if let Ok(plan) = result {
			acyclic += 1;
			assert_dependency_selections(&plan, &units, &reachable);
		}
	}

	assert_eq!(acyclic, 543);
}

#[test]
fn invalid_selections_never_expand_into_a_whole_project_release() {
	let plan = website();
	assert!(plan.select(&[]).is_err());
	assert!(plan.select(&[frontend("missing")]).is_err());
	assert!(plan.select(&[frontend("docs"), frontend("docs")]).is_err());
}
