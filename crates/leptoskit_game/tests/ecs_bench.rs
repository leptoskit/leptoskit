//! Release-mode ECS benchmark suite comparing `leptoskit_game`'s Archetype ECS
//! against industry-standard workloads (matching Bevy / Flecs / Hecs / Godot
//! benchmark shapes).
//!
//! Run with:
//! `cargo test --release -p leptoskit_game --test ecs_bench -- --ignored
//! --nocapture`

use std::hint::black_box;
use std::time::Instant;

use leptoskit_game::Changed;
use leptoskit_game::QueryPlan;
use leptoskit_game::World;

#[derive(Clone, Copy)]
struct Pos([f32; 4]);

#[derive(Clone, Copy)]
struct Vel([f32; 4]);

#[derive(Clone, Copy)]
struct TagA;
#[derive(Clone, Copy)]
struct TagB;
#[derive(Clone, Copy)]
struct TagC;
#[derive(Clone, Copy)]
struct TagD;
#[derive(Clone, Copy)]
struct TagE;
#[derive(Clone, Copy)]
struct TagF;

#[derive(Clone, Copy)]
struct Extra(u64);

fn measure(name: &str, iters: u32, mut f: impl FnMut()) {
	// Warmup
	f();
	let started = Instant::now();
	for _ in 0..iters {
		f();
	}
	let total = started.elapsed();
	let mean_us = total.as_secs_f64() * 1e6 / f64::from(iters);
	println!("{name:<42} {mean_us:>9.2} us/iter  ({iters} iters)");
}

#[test]
#[ignore = "benchmark: run in release mode with --ignored --nocapture"]
fn ecs_performance_suite() {
	println!("\n=== leptoskit_game Archetype ECS Benchmarks (release) ===");

	// 1. Batch spawn 100,000 entities (Pos + Vel).
	measure("spawn_batch_100k (Pos, Vel)", 20, || {
		let mut world = World::new();
		let n = world.spawn_batch_discard(
			(0..100_000).map(|i| (Pos([i as f32, 0.0, 0.0, 1.0]), Vel([1.0, 2.0, 3.0, 0.0]))),
		);
		black_box(n);
	});

	// 2. Single spawn 100,000 entities (Pos + Vel).
	measure("spawn_single_100k (Pos, Vel)", 10, || {
		let mut world = World::new();
		for i in 0..100_000 {
			black_box(world.spawn((Pos([i as f32, 0.0, 0.0, 1.0]), Vel([1.0, 2.0, 3.0, 0.0]))));
		}
	});

	// 3. Simple iteration 100,000 entities (for_each, chunk SIMD, parallel
	//    chunk).
	let mut world = World::new();
	world.spawn_batch_discard((0..100_000).map(|i| {
		(
			Pos([i as f32, 0.0, 0.0, 1.0]),
			Vel([0.016, 0.032, -0.016, 0.0]),
		)
	}));

	measure("iter_100k for_each (&mut Pos, &Vel)", 200, || {
		world.for_each::<(&mut Pos, &Vel)>(|(p, v)| {
			p.0[0] += v.0[0];
			p.0[1] += v.0[1];
			p.0[2] += v.0[2];
		});
		black_box(world.entity_count());
	});

	measure("iter_100k chunk_simd (Pos, Vel)", 200, || {
		world.for_each_chunk_2::<Pos, Vel>(|ps, vs| {
			for (p, v) in ps.iter_mut().zip(vs) {
				p.0[0] += v.0[0];
				p.0[1] += v.0[1];
				p.0[2] += v.0[2];
			}
		});
		black_box(world.entity_count());
	});

	measure("iter_100k par_chunk (Pos, Vel)", 200, || {
		world.par_for_each_chunk_2::<Pos, Vel>(|ps, vs| {
			for (p, v) in ps.iter_mut().zip(vs) {
				p.0[0] += v.0[0];
				p.0[1] += v.0[1];
				p.0[2] += v.0[2];
			}
		});
		black_box(world.entity_count());
	});

	// 4. Fragmented iteration across 64 distinct archetypes (64,000 entities).
	let mut frag = World::new();
	for arch_bits in 0..64_u32 {
		let entities = frag.spawn_batch(
			(0..1000).map(|i| (Pos([i as f32, 0.0, 0.0, 1.0]), Vel([1.0, 0.0, 0.0, 0.0]))),
		);
		for e in entities {
			if arch_bits & 1 != 0 {
				frag.insert_component(e, TagA);
			}
			if arch_bits & 2 != 0 {
				frag.insert_component(e, TagB);
			}
			if arch_bits & 4 != 0 {
				frag.insert_component(e, TagC);
			}
			if arch_bits & 8 != 0 {
				frag.insert_component(e, TagD);
			}
			if arch_bits & 16 != 0 {
				frag.insert_component(e, TagE);
			}
			if arch_bits & 32 != 0 {
				frag.insert_component(e, TagF);
			}
		}
	}
	let mut frag_plan = QueryPlan::new();
	measure("fragmented_64_archetypes_64k (cached)", 200, || {
		frag.for_each_cached::<(&mut Pos, &Vel), ()>(&mut frag_plan, 0, |(p, v)| {
			p.0[0] += v.0[0];
		});
		black_box(frag.archetype_count());
	});

	// 5. Structural add/remove component on 10,000 entities (cached edges).
	let mut mut_world = World::new();
	let handles = mut_world.spawn_batch(
		(0..10_000).map(|i| (Pos([i as f32, 0.0, 0.0, 1.0]), Vel([1.0, 0.0, 0.0, 0.0]))),
	);
	measure("add_remove_component_10k (cached edge)", 50, || {
		for &e in &handles {
			mut_world.insert_component(e, Extra(42));
		}
		for &e in &handles {
			black_box(mut_world.remove_component::<Extra>(e).map(|x| x.0));
		}
	});

	// 6. Change detection skip on 100,000 untouched entities (O(1) column
	//    skip).
	let since = world.change_tick();
	measure("changed_filter_100k_untouched (O(1))", 1000, || {
		let mut count = 0_usize;
		world.for_each_read_filtered::<&Pos, Changed<Pos>>(since, |_| count += 1);
		black_box(count);
	});
}
