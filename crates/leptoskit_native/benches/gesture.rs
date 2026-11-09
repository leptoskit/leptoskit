//! The cost of recognizing a gesture once the tree is already laid out.
//!
//! Each sample is one complete gesture on a 200 by 200 view: a tap (down and
//! up), a pan (down, 16 moves, up), or a pinch (two pointers, 16 moves of the
//! second, both up). The hit test and the arena both run.
//!
//! Run with `cargo bench -p leptoskit_native --bench gesture`.

use leptoskit_native::Gesture;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::PointerPhase;
use leptoskit_native::PointerSample;
use leptoskit_native::components::View;

fn main() {
	divan::main();
}

fn surface() -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::default());
	surface.set_viewport(200.0, 200.0);
	surface.set_root(
		&View().class("w-full h-full").gesture(
			Gesture::tap()
				.on_end(|_| {})
				.simultaneous(Gesture::pan().on_update(|_| {}))
				.simultaneous(Gesture::pinch().on_update(|_| {})),
		),
	);
	surface
}

fn at(id: u64, phase: PointerPhase, x: f32, y: f32, time_ms: u64) -> PointerSample {
	PointerSample::new(id, phase, x, y, time_ms)
}

#[divan::bench]
fn tap(bencher: divan::Bencher) {
	let surface = surface();
	bencher.bench_local(|| {
		surface.pointer(at(1, PointerPhase::Down, 20.0, 20.0, 0));
		surface.pointer(at(1, PointerPhase::Up, 22.0, 21.0, 40));
	});
}

#[divan::bench]
fn pan(bencher: divan::Bencher) {
	let surface = surface();
	bencher.bench_local(|| {
		surface.pointer(at(1, PointerPhase::Down, 20.0, 20.0, 0));

		for step in 1..=16 {
			let distance = 20.0 + step as f32 * 4.0;
			surface.pointer(at(1, PointerPhase::Move, distance, 20.0, step * 8));
		}

		surface.pointer(at(1, PointerPhase::Up, 84.0, 20.0, 160));
	});
}

#[divan::bench]
fn pinch(bencher: divan::Bencher) {
	let surface = surface();
	bencher.bench_local(|| {
		surface.pointer(at(1, PointerPhase::Down, 40.0, 40.0, 0));
		surface.pointer(at(2, PointerPhase::Down, 60.0, 40.0, 8));

		for step in 1..=16 {
			let distance = 60.0 + step as f32 * 4.0;
			surface.pointer(at(2, PointerPhase::Move, distance, 40.0, 8 + step * 8));
		}

		surface.pointer(at(2, PointerPhase::Up, 124.0, 40.0, 160));
		surface.pointer(at(1, PointerPhase::Up, 40.0, 40.0, 168));
	});
}
