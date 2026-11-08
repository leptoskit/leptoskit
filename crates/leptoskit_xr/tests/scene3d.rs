//! A phone session as the camera feed of `leptoskit_3d`'s native views:
//! the scene draws over the session's camera picture. Machines with no GPU
//! adapter skip it.

use std::sync::Arc;

use leptoskit_3d::Gpu;
use leptoskit_3d::Renderer;
use leptoskit_3d::host::CameraFeed;
use leptoskit_3d::host::CameraTarget;
use leptoskit_3d::wgpu;
use leptoskit_xr::host::scene3d::SessionFeed;
use leptoskit_xr::host::*;
use leptoskit_xr::image::*;
use leptoskit_xr::*;

const SIZE: u32 = 32;

struct Still(f64);

impl OrientationSource for Still {
	fn available(&self) -> bool {
		true
	}

	fn start(&mut self) -> Result<(), Error> {
		Ok(())
	}

	fn latest(&mut self) -> Option<(f64, Quat)> {
		self.0 += 1.0 / 60.0;
		Some((self.0, Quat::IDENTITY))
	}

	fn stop(&mut self) {}
}

/// A camera that sees nothing but green.
struct Green;

impl CameraSource for Green {
	fn available(&self) -> bool {
		true
	}

	fn start(&mut self) -> Result<(), Error> {
		Ok(())
	}

	fn latest(&mut self) -> Option<Arc<CapturedImage>> {
		let green: Vec<u8> = std::iter::repeat_n([0, 255, 0, 255], 16)
			.flatten()
			.collect();
		Some(Arc::new(CapturedImage {
			time: 1.0,
			width: 4,
			height: 4,
			pixels: CameraPixels::Rgba(ImagePlane {
				width: 4,
				height: 4,
				row_stride: 16,
				data: green.into(),
			}),
			sensor_quarter_turns: 0,
			intrinsics: Intrinsics {
				focal: [4.0, 4.0],
				principal: [2.0, 2.0],
				size: [4, 4],
			},
		}))
	}

	fn stop(&mut self) {}
}

#[test]
fn a_running_session_paints_its_camera_behind_the_scene() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(error) if error.is_unavailable() => {
			eprintln!("skipping: {error}");
			return;
		}
		Err(error) => panic!("{error}"),
	};
	let mut renderer = Renderer::new(&gpu);
	let texture = renderer.offscreen(SIZE, SIZE).unwrap();
	let color = texture.create_view(&wgpu::TextureViewDescriptor::default());
	let target = CameraTarget {
		color: &color,
		format: wgpu::TextureFormat::Rgba8Unorm,
		size: [SIZE, SIZE],
		quarter_turns: 0,
	};

	let runtime = XrRuntime::new(OrientationBackend::new(Still(0.0)).with_camera(Green));
	let feed = SessionFeed::new(Arc::clone(&runtime));
	let mut painter = feed.painter();

	// No session: the view draws as it would without a feed.
	assert!(!feed.live());
	assert!(painter.draw(&mut renderer, &target).is_none());

	runtime
		.start(SessionConfig::new(Mode::Orientation).prefer(Features::CAMERA_IMAGE))
		.unwrap();
	runtime.update(runtime.now()).unwrap();
	assert!(feed.live());
	painter
		.draw(&mut renderer, &target)
		.expect("a session with a picture paints")
		.unwrap();
	let pixels = renderer.read_back(&texture).unwrap();
	let middle = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
	assert_eq!(&pixels[middle..middle + 4], &[0, 255, 0, 255]);

	runtime.stop();
	assert!(!feed.live());
	assert!(painter.draw(&mut renderer, &target).is_none());
}
