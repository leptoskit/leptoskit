//! Comprehensive integration tests for the `leptoskit_game` Archetype ECS,
//! queries, change detection, Godot-style names/groups/signals, events,
//! commands, schedules, plugins, and finite states.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_game::Added;
use leptoskit_game::Changed;
use leptoskit_game::Entity;
use leptoskit_game::EventCursor;
use leptoskit_game::GameApp;
use leptoskit_game::GamePlugin;
use leptoskit_game::Mut;
use leptoskit_game::NextState;
use leptoskit_game::QueryPlan;
use leptoskit_game::Schedule;
use leptoskit_game::Stage;
use leptoskit_game::State;
use leptoskit_game::SystemAccess;
use leptoskit_game::SystemConfig;
use leptoskit_game::Time;
use leptoskit_game::With;
use leptoskit_game::Without;
use leptoskit_game::World;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pos {
	x: f32,
	y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Vel {
	dx: f32,
	dy: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Health(i32);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Armor(i32);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Enemy;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Frozen;

#[test]
fn entity_generations_prevent_stale_access() {
	let mut world = World::new();
	let first = world.spawn((Pos { x: 1.0, y: 2.0 }, Health(100)));
	assert!(world.is_alive(first));
	assert_eq!(world.entity_count(), 1);
	assert_eq!(world.get_component::<Health>(first), Some(&Health(100)));

	assert!(world.despawn(first));
	assert!(!world.is_alive(first));
	assert!(!world.despawn(first));
	assert_eq!(world.get_component::<Health>(first), None);
	assert!(!world.insert_component(first, Armor(5)));

	let recycled = world.spawn((Pos { x: 9.0, y: 9.0 },));
	assert_eq!(recycled.index(), first.index());
	assert_ne!(recycled.generation(), first.generation());
	assert!(world.is_alive(recycled));
	assert!(!world.is_alive(first));
	assert_eq!(world.get_component::<Pos>(first), None);
	assert_eq!(
		world.get_component::<Pos>(recycled),
		Some(&Pos { x: 9.0, y: 9.0 })
	);
}

#[test]
fn archetype_transitions_preserve_components_and_reuse_cached_edges() {
	let mut world = World::new();
	let entities: Vec<Entity> = (0..128)
		.map(|i| {
			world.spawn((
				Pos {
					x: i as f32,
					y: 0.0,
				},
				Vel { dx: 1.0, dy: 2.0 },
			))
		})
		.collect();

	// Add Armor to all even entities -> triggers archetype migration.
	for (i, &entity) in entities.iter().enumerate() {
		if i % 2 == 0 {
			assert!(world.insert_component(entity, Armor(i as i32)));
		}
	}
	assert_eq!(world.entity_count(), 128);

	// Overwrite Armor in place (no migration).
	assert!(world.insert_component(entities[0], Armor(999)));
	assert_eq!(world.get_component::<Armor>(entities[0]), Some(&Armor(999)));

	// Verify all components survived swap-removes cleanly.
	for (i, &entity) in entities.iter().enumerate() {
		assert_eq!(
			world.get_component::<Pos>(entity),
			Some(&Pos {
				x: i as f32,
				y: 0.0,
			})
		);
		assert_eq!(
			world.get_component::<Vel>(entity),
			Some(&Vel { dx: 1.0, dy: 2.0 })
		);
		let expected_armor = if i == 0 {
			Some(&Armor(999))
		} else if i % 2 == 0 {
			Some(&Armor(i as i32))
		} else {
			None
		};
		assert_eq!(world.get_component::<Armor>(entity), expected_armor);
	}

	// Remove Armor from half of the armored entities.
	for (i, &entity) in entities.iter().enumerate() {
		if i % 4 == 0 {
			let removed = world.remove_component::<Armor>(entity);
			assert!(removed.is_some());
			assert!(!world.has_component::<Armor>(entity));
			assert_eq!(
				world.get_component::<Pos>(entity),
				Some(&Pos {
					x: i as f32,
					y: 0.0,
				})
			);
		}
	}
}

#[test]
fn queries_filters_chunk_and_parallel_iteration() {
	let mut world = World::new();
	world.spawn_batch((0..5000).map(|i| {
		(
			Pos {
				x: i as f32,
				y: 0.0,
			},
			Vel { dx: 2.0, dy: -1.0 },
			Enemy,
		)
	}));
	world.spawn_batch((0..500).map(|i| {
		(
			Pos {
				x: i as f32,
				y: 10.0,
			},
			Vel { dx: 5.0, dy: 0.0 },
			Enemy,
			Frozen,
		)
	}));

	// Move only non-frozen enemies.
	world.for_each_filtered::<(&mut Pos, &Vel), (With<Enemy>, Without<Frozen>)>(0, |(p, v)| {
		p.x += v.dx;
		p.y += v.dy;
	});

	let mut moved = 0_usize;
	let mut frozen_untouched = 0_usize;
	world.for_each_read_filtered::<&Pos, (With<Enemy>, Without<Frozen>)>(0, |p| {
		assert!((p.y - (-1.0)).abs() < f32::EPSILON);
		moved += 1;
	});
	world.for_each_read_filtered::<&Pos, With<Frozen>>(0, |p| {
		assert!((p.y - 10.0).abs() < f32::EPSILON);
		frozen_untouched += 1;
	});
	assert_eq!(moved, 5000);
	assert_eq!(frozen_untouched, 500);

	// SIMD chunk path + parallel chunk path.
	world.for_each_chunk_2::<Pos, Vel>(|positions, velocities| {
		for (p, v) in positions.iter_mut().zip(velocities) {
			p.x += v.dx;
		}
	});
	world.par_for_each_chunk_2::<Pos, Vel>(|positions, velocities| {
		for (p, v) in positions.iter_mut().zip(velocities) {
			p.y += v.dy;
		}
	});

	let mut plan = QueryPlan::new();
	let mut total = 0_usize;
	world.for_each_read_cached::<(&Pos, &Vel), With<Enemy>>(&mut plan, 0, |_| {
		total += 1;
	});
	assert_eq!(total, 5500);

	// Spawning a new archetype incrementally refreshes the cached QueryPlan.
	world.spawn((
		Pos { x: 0.0, y: 0.0 },
		Vel { dx: 0.0, dy: 0.0 },
		Enemy,
		Health(10),
	));
	let mut total_after = 0_usize;
	world.for_each_read_cached::<(&Pos, &Vel), With<Enemy>>(&mut plan, 0, |_| {
		total_after += 1;
	});
	assert_eq!(total_after, 5501);
}

#[test]
fn change_detection_and_lazy_mut_skip_untouched_entities() {
	let mut world = World::new();
	let a = world.spawn((Pos { x: 0.0, y: 0.0 }, Health(100)));
	let b = world.spawn((Pos { x: 1.0, y: 0.0 }, Health(100)));
	let since_spawn = world.change_tick();

	// Nothing added or changed after `since_spawn` yet.
	let mut changed = 0;
	world.for_each_read_filtered::<&Health, Changed<Health>>(since_spawn, |_| changed += 1);
	assert_eq!(changed, 0);

	// Advance tick and read with `Mut` without mutating `b` (using
	// `set_if_neq`).
	world.increment_change_tick();
	world.for_each::<Mut<'_, Health>>(|mut hp| {
		let _ = hp.set_if_neq(Health(100));
	});
	world.for_each_read_filtered::<&Health, Changed<Health>>(since_spawn, |_| changed += 1);
	assert_eq!(changed, 0);

	// Mutate only `a`.
	world.get_component_mut::<Health>(a).unwrap().0 = 50;
	let mut changed_entities = Vec::new();
	world.for_each_read_filtered::<(Entity, &Health), Changed<Health>>(since_spawn, |(e, _)| {
		changed_entities.push(e);
	});
	assert_eq!(changed_entities, vec![a]);
	assert_ne!(b, a);

	// Spawn a third entity on a newer tick -> `Added<Health>` finds only `c`.
	let since_second = world.increment_change_tick();
	world.increment_change_tick();
	let c = world.spawn((Pos { x: 2.0, y: 0.0 }, Health(75)));
	let mut added_entities = Vec::new();
	world.for_each_read_filtered::<(Entity, &Health), Added<Health>>(since_second, |(e, _)| {
		added_entities.push(e);
	});
	assert_eq!(added_entities, vec![c]);
}

#[test]
fn godot_names_groups_and_signal_observers() {
	#[derive(Debug, Clone, Copy)]
	struct Damaged(i32);

	let mut world = World::new();
	let player = world.spawn_named("player", (Health(100),));
	let goblin = world.spawn_named("goblin", (Health(30), Enemy));
	world.add_to_group(goblin, "enemies");
	world.add_to_group(goblin, "enemies"); // idempotent

	assert_eq!(world.named("player"), Some(player));
	assert_eq!(world.name_of(player), Some("player"));
	assert_eq!(world.group("enemies"), &[goblin]);

	let total_damage = Arc::new(AtomicUsize::new(0));
	let counter_global = Arc::clone(&total_damage);
	let global_obs = world.observe::<Damaged>(move |trigger, _world| {
		counter_global.fetch_add(trigger.event.0 as usize, Ordering::Relaxed);
	});

	let player_hits = Arc::new(AtomicUsize::new(0));
	let counter_player = Arc::clone(&player_hits);
	world.observe_entity::<Damaged>(player, move |trigger, world| {
		counter_player.fetch_add(1, Ordering::Relaxed);
		if let Some(hp) = world.get_component_mut::<Health>(player) {
			hp.0 -= trigger.event.0;
		}
	});

	world.trigger_for(player, Damaged(25));
	world.trigger_for(goblin, Damaged(10));
	assert_eq!(world.get_component::<Health>(player), Some(&Health(75)));
	assert_eq!(player_hits.load(Ordering::Relaxed), 1);
	assert_eq!(total_damage.load(Ordering::Relaxed), 35);

	// Despawning cleans up names, groups, and entity-scoped observers.
	assert!(world.despawn(goblin));
	assert_eq!(world.named("goblin"), None);
	assert_eq!(world.group("enemies"), []);

	assert!(world.despawn(player));
	world.trigger_for(player, Damaged(5));
	assert_eq!(player_hits.load(Ordering::Relaxed), 1);
	assert!(world.remove_observer(global_obs));
	world.trigger(Damaged(99));
	assert_eq!(total_damage.load(Ordering::Relaxed), 40);
}

#[test]
fn double_buffered_events_and_commands() {
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	struct CoinCollected(u32);

	let mut world = World::new();
	world.register_event::<CoinCollected>();
	let mut cursor_a = EventCursor::<CoinCollected>::new();
	let mut cursor_b = EventCursor::<CoinCollected>::new();

	world.send_event(CoinCollected(1));
	world.send_event(CoinCollected(5));
	let seen_a: Vec<_> = cursor_a
		.read(world.events::<CoinCollected>().unwrap())
		.copied()
		.collect();
	assert_eq!(seen_a, vec![CoinCollected(1), CoinCollected(5)]);

	// Rotate buffer: cursor_b (late reader) still sees frame 1's events!
	world.update_events();
	world.send_event(CoinCollected(10));
	let seen_b: Vec<_> = cursor_b
		.read(world.events::<CoinCollected>().unwrap())
		.copied()
		.collect();
	assert_eq!(
		seen_b,
		vec![CoinCollected(1), CoinCollected(5), CoinCollected(10)]
	);

	// Rotate again: frame 1 drops out, cursor_a only sees frame 2's
	// CoinCollected(10).
	world.update_events();
	let seen_a2: Vec<_> = cursor_a
		.read(world.events::<CoinCollected>().unwrap())
		.copied()
		.collect();
	assert_eq!(seen_a2, vec![CoinCollected(10)]);

	// Deferred commands.
	let e = world.spawn((Health(10),));
	world.commands().insert(e, Armor(4));
	world
		.commands()
		.spawn_named("loot", (Pos { x: 3.0, y: 4.0 },));
	assert!(!world.has_component::<Armor>(e));
	world.flush_commands();
	assert_eq!(world.get_component::<Armor>(e), Some(&Armor(4)));
	assert!(world.named("loot").is_some());
}

#[test]
fn game_app_plugins_schedules_waves_and_states() {
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
	enum Mode {
		Menu,
		Playing,
		Paused,
	}

	struct Log(Vec<&'static str>);

	struct PhysicsPlugin;
	impl GamePlugin for PhysicsPlugin {
		fn build(&self, app: &mut GameApp) {
			app.add_system(
				Stage::FixedUpdate,
				SystemConfig::new("integrate", |world| {
					let dt = world.get::<Time>().map_or(1.0 / 60.0, |t| t.fixed_dt);
					world.for_each::<(&mut Pos, &Vel)>(|(p, v)| {
						p.x += v.dx * dt;
						p.y += v.dy * dt;
					});
				})
				.in_state(Mode::Playing),
			);
		}
	}

	let mut app = GameApp::new().with_fixed_hz(60);
	app.insert_resource(Log(Vec::new()));
	app.init_state(Mode::Menu);
	app.add_plugin(PhysicsPlugin);

	app.on_exit(Mode::Menu, |world| {
		world.get_mut::<Log>().unwrap().0.push("exit:menu");
	});
	app.on_enter(Mode::Playing, |world| {
		world.get_mut::<Log>().unwrap().0.push("enter:playing");
		world.spawn_named("hero", (Pos { x: 0.0, y: 0.0 }, Vel { dx: 60.0, dy: 0.0 }));
	});
	app.on_enter(Mode::Paused, |world| {
		world.get_mut::<Log>().unwrap().0.push("enter:paused");
	});

	// Ordering constraints: "second" must run after "first" and before "third".
	app.add_system(
		Stage::Update,
		SystemConfig::new("third", |world| {
			world.get_mut::<Log>().unwrap().0.push("third");
		})
		.after("second"),
	);
	app.add_system(
		Stage::Update,
		SystemConfig::new("first", |world| {
			world.get_mut::<Log>().unwrap().0.push("first");
		})
		.before("second"),
	);
	app.add_system(
		Stage::Update,
		SystemConfig::new("second", |world| {
			world.get_mut::<Log>().unwrap().0.push("second");
		}),
	);

	// Step while in Menu -> hero not spawned yet.
	app.step_fixed();
	assert_eq!(app.world().named("hero"), None);
	assert_eq!(
		app.world().get::<Log>().unwrap().0,
		vec!["first", "second", "third"]
	);

	// Transition to Playing.
	app.world_mut()
		.get_mut::<NextState<Mode>>()
		.unwrap()
		.set(Mode::Playing);
	// Run 1/30s (two 60Hz fixed ticks in one frame).
	app.update(1.0 / 30.0);
	let hero = app.world().named("hero").expect("hero spawned on enter");
	let pos = app.world().get_component::<Pos>(hero).unwrap();
	assert!((pos.x - 2.0).abs() < 1e-4, "expected 2.0, got {}", pos.x);
	assert_eq!(
		app.world().get::<State<Mode>>().unwrap().get(),
		&Mode::Playing
	);

	// Transition to Paused -> hero stops integrating.
	app.world_mut()
		.get_mut::<NextState<Mode>>()
		.unwrap()
		.set(Mode::Paused);
	app.step_fixed();
	let paused_x = app.world().get_component::<Pos>(hero).unwrap().x;
	app.step_fixed();
	assert_eq!(app.world().get_component::<Pos>(hero).unwrap().x, paused_x);

	// Conflict wave planner puts independent readers in 1 wave and conflicting
	// writers in separate waves.
	let mut schedule = Schedule::default();
	schedule.add_system(
		SystemConfig::new("read_pos", |_| {}).with_access(SystemAccess::default().read::<Pos>()),
	);
	schedule.add_system(
		SystemConfig::new("read_pos_2", |_| {}).with_access(SystemAccess::default().read::<Pos>()),
	);
	schedule.add_system(
		SystemConfig::new("write_vel", |_| {}).with_access(SystemAccess::default().write::<Vel>()),
	);
	assert_eq!(schedule.wave_count(), 1);

	schedule.add_system(
		SystemConfig::new("write_pos", |_| {}).with_access(SystemAccess::default().write::<Pos>()),
	);
	assert_eq!(schedule.wave_count(), 2);
}

#[test]
#[should_panic(expected = "cycle detected")]
fn schedule_rejects_ordering_cycles() {
	let mut schedule = Schedule::default();
	schedule.add_system(SystemConfig::new("a", |_| {}).after("b"));
	schedule.add_system(SystemConfig::new("b", |_| {}).after("a"));
	schedule.build();
}
