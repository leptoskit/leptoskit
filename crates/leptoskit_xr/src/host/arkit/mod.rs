//! ARKit: world tracking, planes, LiDAR depth, light estimation, raycasts,
//! and anchors on iOS.
//!
//! The session is polled: each update reads `-[ARSession currentFrame]`, so
//! no delegate or dispatch queue is involved, and whichever thread drives
//! the runtime may call it. The camera picture stays in its `CVPixelBuffer`
//! for the renderer to wrap in Metal textures without a copy; the depth
//! picture, at most 256 × 192, is copied for hit tests.

use std::collections::HashMap;
use std::sync::Arc;

use leptoskit_bridge::Error;
use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_ar_kit::*;
use objc2_av_foundation::AVAuthorizationStatus;
use objc2_av_foundation::AVCaptureDevice;
use objc2_av_foundation::AVMediaTypeVideo;
use objc2_core_foundation::CGSize;
use objc2_core_video::CVPixelBuffer;
use objc2_ui_kit::UIInterfaceOrientation;

use super::Backend;
use super::Viewport;
use super::XrFrame;
use crate::hit_test;
use crate::image::*;
use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Quat;
use crate::math::Vec3;
use crate::types::*;

mod simd;

/// The clip planes ARKit's projection is asked for; only its angles are
/// kept, so these never reach the renderer.
const NEAR: f64 = 0.05;
const FAR: f64 = 100.0;
/// ARKit's neutral ambient intensity, in lumens.
const NEUTRAL_LUMENS: f32 = 1000.0;
/// `ARRaycastTargetExistingPlaneGeometry`, `…ExistingPlaneInfinite`, and
/// `…EstimatedPlane`.
const RAYCAST_PLANE: isize = 0;
const RAYCAST_INFINITE: isize = 1;
const RAYCAST_ESTIMATED: isize = 2;
/// `ARRaycastTargetAlignmentAny`.
const ALIGN_ANY: isize = 2;

/// A camera picture ARKit captured: bi-planar 4:2:0 YCbCr, full range.
#[derive(Debug, Clone)]
pub struct PixelBuffer(Retained<CVPixelBuffer>);

// SAFETY: A `CVPixelBuffer` is reference counted atomically, and ARKit never
// writes to a buffer after handing it out; readers lock it first.
unsafe impl Send for PixelBuffer {}
// SAFETY: As above.
unsafe impl Sync for PixelBuffer {}

impl PixelBuffer {
	/// The `CVPixelBufferRef`, for `CVMetalTextureCacheCreateTextureFromImage`.
	/// Valid while this value lives.
	#[must_use]
	pub fn as_ptr(&self) -> *mut std::ffi::c_void {
		Retained::as_ptr(&self.0).cast_mut().cast()
	}
}

/// ARKit's world tracking.
pub struct ArKitBackend {
	session: Option<Retained<ARSession>>,
	features: Features,
	viewport: Viewport,
	last_time: f64,
	planes: Ids,
	anchors: HashMap<u64, Retained<ARAnchor>>,
	next_anchor: u64,
}

impl std::fmt::Debug for ArKitBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ArKitBackend")
			.field("running", &self.session.is_some())
			.field("features", &self.features)
			.finish_non_exhaustive()
	}
}

// SAFETY: `ARSession` and its frames may be used from any thread; the
// runtime uses the backend from one thread at a time.
unsafe impl Send for ArKitBackend {}

/// Stable numbers for ARKit's anchor identifiers.
#[derive(Debug, Default)]
struct Ids {
	known: HashMap<String, u64>,
	next: u64,
}

impl Ids {
	fn of(&mut self, anchor: &ARAnchor) -> u64 {
		// SAFETY: Property reads on a live anchor.
		let key = unsafe { anchor.identifier().UUIDString() }.to_string();
		let next = &mut self.next;

		*self.known.entry(key).or_insert_with(|| {
			*next += 1;
			*next
		})
	}
}

impl Default for ArKitBackend {
	fn default() -> Self {
		Self::new()
	}
}

impl ArKitBackend {
	/// A backend with no session running.
	#[must_use]
	pub fn new() -> Self {
		Self {
			session: None,
			features: Features::NONE,
			viewport: Viewport::default(),
			last_time: f64::NAN,
			planes: Ids::default(),
			anchors: HashMap::new(),
			next_anchor: 0,
		}
	}

	fn orientation(&self) -> UIInterfaceOrientation {
		match self.viewport.quarter_turns % 4 {
			1 => UIInterfaceOrientation::LandscapeRight,
			2 => UIInterfaceOrientation::PortraitUpsideDown,
			3 => UIInterfaceOrientation::LandscapeLeft,
			_ => UIInterfaceOrientation::Portrait,
		}
	}

	fn size(&self) -> CGSize {
		CGSize::new(
			f64::from(self.viewport.width.max(1)),
			f64::from(self.viewport.height.max(1)),
		)
	}

	fn configuration(features: Features) -> Retained<ARWorldTrackingConfiguration> {
		// SAFETY: Configures a fresh configuration before it is run.
		unsafe {
			let configuration = ARWorldTrackingConfiguration::new();
			let mut detection = ARPlaneDetection::empty();

			if features.contains(Features::PLANES_HORIZONTAL) {
				detection |= ARPlaneDetection::Horizontal;
			}

			if features.contains(Features::PLANES_VERTICAL) {
				detection |= ARPlaneDetection::Vertical;
			}

			configuration.setPlaneDetection(detection);
			configuration.setLightEstimationEnabled(features.contains(Features::LIGHT_ESTIMATION));

			if features.contains(Features::DEPTH) {
				configuration.setFrameSemantics(
					ARFrameSemantics::SceneDepth | ARFrameSemantics::SmoothedSceneDepth,
				);
			}

			configuration
		}
	}

	fn frame(&mut self, frame: &ARFrame) -> XrFrame {
		// SAFETY: Property reads on the current frame and its camera.
		let (time, camera) = unsafe { (frame.timestamp(), frame.camera()) };
		let orientation = self.orientation().0;
		let camera_ptr = Retained::as_ptr(&camera).cast::<AnyObject>();
		// SAFETY: `camera` is a live `ARCamera`.
		let view_matrix = unsafe { simd::view_matrix(camera_ptr, orientation) };
		let viewer = view_matrix.pose().inverse();
		let size = self.size();
		// SAFETY: As above.
		let projection = unsafe {
			simd::projection_matrix(
				camera_ptr,
				orientation,
				(size.width, size.height),
				NEAR,
				FAR,
			)
		};
		let projection = Fov::from_matrix(&projection.to_array())
			.map_or(Projection::Matrix(projection.to_array()), Projection::Fov);
		let display = self.display_transform(frame);

		XrFrame {
			time,
			tracking: tracking(&camera),
			viewer,
			views: vec![View {
				pose: viewer,
				projection,
			}],
			camera_image: Some(camera_image(frame, display)),
			depth: self
				.features
				.contains(Features::DEPTH)
				.then(|| depth(frame, display))
				.flatten(),
			planes: self.planes_of(frame).into(),
			planes_revision: 0,
			light: self
				.features
				.contains(Features::LIGHT_ESTIMATION)
				.then(|| light(frame))
				.flatten(),
			anchors: self.anchors_of(frame),
			inputs: Vec::new(),
		}
	}

	/// Maps the view to the captured image: ARKit's transform maps the image
	/// to the view, so it is inverted.
	fn display_transform(&self, frame: &ARFrame) -> Affine2 {
		// SAFETY: A pure computation on the frame.
		let transform = unsafe {
			frame.displayTransformForOrientation_viewportSize(self.orientation(), self.size())
		};
		let image_to_view = [
			transform.a as f32,
			transform.b as f32,
			transform.c as f32,
			transform.d as f32,
			transform.tx as f32,
			transform.ty as f32,
		];

		invert_affine(&image_to_view).unwrap_or(IDENTITY_AFFINE)
	}

	fn planes_of(&mut self, frame: &ARFrame) -> Vec<Plane> {
		// SAFETY: Property read.
		let anchors = unsafe { frame.anchors() };

		anchors
			.iter()
			.filter_map(|anchor| {
				let plane = anchor.downcast_ref::<ARPlaneAnchor>()?;
				Some(self.plane(plane))
			})
			.collect()
	}

	fn plane(&mut self, anchor: &ARPlaneAnchor) -> Plane {
		let pointer = std::ptr::from_ref(anchor).cast::<AnyObject>();
		// SAFETY: `anchor` is a live `ARPlaneAnchor`.
		let (transform, center) = unsafe {
			(
				simd::matrix(pointer, objc2::sel!(transform)).pose(),
				simd::vec3(simd::float3_of(pointer, objc2::sel!(center))),
			)
		};
		// SAFETY: Property reads.
		let (extent, alignment, classification, geometry) = unsafe {
			(
				anchor.planeExtent(),
				anchor.alignment(),
				anchor.classification(),
				anchor.geometry(),
			)
		};
		// SAFETY: As above.
		let (width, height, turn) =
			unsafe { (extent.width(), extent.height(), extent.rotationOnYAxis()) };
		let local = Pose::new(center, Quat::from_axis_angle(Vec3::Y, turn));
		let pose = transform.then(local);
		let normal = pose.transform_vector(Vec3::Y);
		let orientation = if alignment == ARPlaneAnchorAlignment::Vertical {
			PlaneOrientation::Vertical
		} else if normal.y >= 0.0 {
			PlaneOrientation::HorizontalUp
		} else {
			PlaneOrientation::HorizontalDown
		};

		Plane {
			id: self.planes.of(anchor),
			orientation,
			label: label(classification),
			pose,
			extent: [width, height],
			polygon: boundary(&geometry, local),
		}
	}

	fn anchors_of(&mut self, frame: &ARFrame) -> Vec<Anchor> {
		let tracking = {
			// SAFETY: Property read.
			let camera = unsafe { frame.camera() };
			tracking(&camera)
		};

		self.anchors
			.iter()
			.map(|(&id, anchor)| {
				let pointer = Retained::as_ptr(anchor).cast::<AnyObject>();
				// SAFETY: `anchor` is a live `ARAnchor`.
				let pose = unsafe { simd::matrix(pointer, objc2::sel!(transform)) }.pose();
				Anchor { id, pose, tracking }
			})
			.collect()
	}

	fn raycast(&mut self, session: &ARSession, ray: crate::math::Ray, target: isize) -> Vec<Hit> {
		let kind = match target {
			RAYCAST_PLANE => HitKind::Plane,
			RAYCAST_INFINITE => HitKind::InfinitePlane,
			_ => HitKind::Estimated,
		};
		// SAFETY: Allocates and initialises a query, then raycasts it; the
		// query is released when `query` drops.
		let results = unsafe {
			let allocated: *mut AnyObject = objc2::msg_send![ARRaycastQuery::class(), alloc];
			let query = simd::init_raycast(
				allocated,
				simd::float3(ray.origin),
				simd::float3(ray.direction),
				target,
				ALIGN_ANY,
			);
			let Some(query) = Retained::from_raw(query.cast::<ARRaycastQuery>()) else {
				return Vec::new();
			};
			session.raycast(&query)
		};

		results
			.iter()
			.map(|result| {
				let pointer = Retained::as_ptr(&result).cast::<AnyObject>();
				// SAFETY: `result` is a live `ARRaycastResult`.
				let pose = unsafe { simd::matrix(pointer, objc2::sel!(worldTransform)) }.pose();
				// SAFETY: Property read.
				let plane = unsafe { result.anchor() }.map(|anchor| self.planes.of(&anchor));

				Hit {
					distance: (pose.position - ray.origin).length(),
					pose,
					kind,
					plane: plane.filter(|_| kind != HitKind::Estimated),
				}
			})
			.collect()
	}
}

fn camera_allowed() -> Option<bool> {
	// SAFETY: Reads the app's camera authorization, never prompting.
	let status = unsafe {
		let video = AVMediaTypeVideo?;
		AVCaptureDevice::authorizationStatusForMediaType(video)
	};

	match status {
		AVAuthorizationStatus::Authorized => Some(true),
		AVAuthorizationStatus::NotDetermined => None,
		_ => Some(false),
	}
}

fn tracking(camera: &ARCamera) -> TrackingState {
	// SAFETY: Property reads.
	let (state, reason) = unsafe { (camera.trackingState(), camera.trackingStateReason()) };

	match state {
		ARTrackingState::Normal => TrackingState::Normal,
		ARTrackingState::Limited => {
			TrackingState::Limited(match reason {
				ARTrackingStateReason::Initializing => LimitedReason::Initializing,
				ARTrackingStateReason::ExcessiveMotion => LimitedReason::ExcessiveMotion,
				ARTrackingStateReason::InsufficientFeatures => LimitedReason::InsufficientFeatures,
				ARTrackingStateReason::Relocalizing => LimitedReason::Relocalizing,
				_ => LimitedReason::Unknown,
			})
		}
		_ => TrackingState::NotAvailable,
	}
}

fn label(classification: ARPlaneClassification) -> PlaneLabel {
	match classification {
		ARPlaneClassification::Wall => PlaneLabel::Wall,
		ARPlaneClassification::Floor => PlaneLabel::Floor,
		ARPlaneClassification::Ceiling => PlaneLabel::Ceiling,
		ARPlaneClassification::Table => PlaneLabel::Table,
		ARPlaneClassification::Seat => PlaneLabel::Seat,
		ARPlaneClassification::Window => PlaneLabel::Window,
		ARPlaneClassification::Door => PlaneLabel::Door,
		_ => PlaneLabel::Unknown,
	}
}

/// A plane's outline in the plane's own x–z, from ARKit's boundary, which is
/// in the anchor's space.
fn boundary(geometry: &ARPlaneGeometry, local: Pose) -> Vec<[f32; 2]> {
	let pointer = std::ptr::from_ref(geometry).cast::<AnyObject>();
	// SAFETY: Property read.
	let count = unsafe { geometry.boundaryVertexCount() };
	// SAFETY: `boundaryVertices` points at `count` `simd_float3`s, which live
	// as long as the geometry.
	let vertices = unsafe {
		let vertices = simd::pointer(pointer, objc2::sel!(boundaryVertices)).cast::<simd::Float3>();

		if vertices.is_null() {
			return Vec::new();
		}

		std::slice::from_raw_parts(vertices, count)
	};
	let to_plane = local.inverse();
	let points: Vec<[f32; 2]> = vertices
		.iter()
		.map(|vertex| {
			let point = to_plane.transform_point(simd::vec3(*vertex));
			[point.x, point.z]
		})
		.collect();

	thin_polygon(&points)
}

fn camera_image(frame: &ARFrame, display_transform: Affine2) -> CameraImage {
	// SAFETY: Property read; the buffer is retained.
	let buffer = unsafe { frame.capturedImage() };
	let width = objc2_core_video::CVPixelBufferGetWidth(&buffer) as u32;
	let height = objc2_core_video::CVPixelBufferGetHeight(&buffer) as u32;

	CameraImage {
		width,
		height,
		display_transform,
		pixels: CameraPixels::Native(NativeImage::PixelBuffer(PixelBuffer(buffer))),
	}
}

/// Copies a locked pixel buffer's rows.
fn copy_rows(buffer: &CVPixelBuffer) -> Option<(u32, u32, u32, Arc<[u8]>)> {
	use objc2_core_video::*;

	// SAFETY: Locks the buffer for reading, copies its rows, and unlocks it.
	unsafe {
		if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
			return None;
		}

		let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
		let stride = CVPixelBufferGetBytesPerRow(buffer);
		let height = CVPixelBufferGetHeight(buffer);
		let width = CVPixelBufferGetWidth(buffer);
		let copy =
			(!base.is_null()).then(|| Arc::from(std::slice::from_raw_parts(base, stride * height)));
		CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly);
		Some((width as u32, height as u32, stride as u32, copy?))
	}
}

/// The smoothed LiDAR depth where the device has it, with ARKit's three
/// confidence levels spread over 0–255.
fn depth(frame: &ARFrame, display_transform: Affine2) -> Option<DepthImage> {
	// SAFETY: Property reads.
	let data = unsafe { frame.smoothedSceneDepth().or_else(|| frame.sceneDepth()) }?;
	// SAFETY: As above.
	let (map, confidence) = unsafe { (data.depthMap(), data.confidenceMap()) };
	let (width, height, row_stride, bytes) = copy_rows(&map)?;
	let confidence = confidence
		.and_then(|confidence| copy_rows(&confidence))
		.map(|(width, height, row_stride, bytes)| {
			let scaled: Vec<u8> = bytes
				.iter()
				.map(|level| {
					match level {
						0 => 0,
						1 => 128,
						_ => 255,
					}
				})
				.collect();
			ImagePlane {
				width,
				height,
				row_stride,
				data: scaled.into(),
			}
		});

	Some(DepthImage {
		width,
		height,
		format: DepthFormat::Meters32,
		data: bytes,
		row_stride,
		confidence,
		display_transform,
	})
}

fn light(frame: &ARFrame) -> Option<LightEstimate> {
	// SAFETY: Property reads.
	let estimate = unsafe { frame.lightEstimate() }?;
	// SAFETY: As above.
	let (lumens, kelvin) = unsafe {
		(
			estimate.ambientIntensity(),
			estimate.ambientColorTemperature(),
		)
	};

	Some(LightEstimate {
		intensity: lumens as f32 / NEUTRAL_LUMENS,
		color_temperature: kelvin as f32,
		main_light_direction: None,
	})
}

impl Backend for ArKitBackend {
	fn capabilities(&self) -> Capabilities {
		// These are class methods each configuration overrides, so they are
		// sent to the world-tracking class rather than its superclass.
		let class = ARWorldTrackingConfiguration::class();
		// SAFETY: Class methods reporting device support.
		let (supported, depth): (bool, bool) = unsafe {
			(
				objc2::msg_send![class, isSupported],
				objc2::msg_send![class, supportsFrameSemantics: ARFrameSemantics::SceneDepth],
			)
		};

		if !supported {
			return Capabilities::NONE;
		}

		let features = Features::PLANES
			| Features::LIGHT_ESTIMATION
			| Features::HIT_TEST
			| Features::ANCHORS
			| Features::CAMERA_IMAGE;

		Capabilities::NONE.with(
			Mode::PhoneAr,
			if depth {
				features | Features::DEPTH
			} else {
				features
			},
		)
	}

	fn prepare(&mut self, mode: Mode, _install: bool) -> Availability {
		if !self.capabilities().supports(mode) {
			return Availability::Unsupported;
		}

		match camera_allowed() {
			Some(true) => Availability::Ready,
			_ => Availability::NeedsPermission,
		}
	}

	fn start(
		&mut self,
		config: &SessionConfig,
		features: Features,
	) -> Result<ReferenceSpace, Error> {
		if camera_allowed() == Some(false) {
			return Err(Error::PermissionDenied);
		}

		// SAFETY: Makes a session and runs it with a fresh configuration.
		let session = unsafe { ARSession::new() };
		// SAFETY: As above.
		unsafe { session.runWithConfiguration(&Self::configuration(features)) };
		self.session = Some(session);
		self.features = features;
		self.last_time = f64::NAN;
		// ARKit's origin is where the session started, gravity-aligned; it
		// has no floor-relative origin, so a floor request gets world space.
		Ok(match config.reference_space {
			ReferenceSpace::Local => ReferenceSpace::Local,
			ReferenceSpace::LocalFloor | ReferenceSpace::World => ReferenceSpace::World,
		})
	}

	fn configure(
		&mut self,
		config: &SessionConfig,
		features: Features,
	) -> Result<ReferenceSpace, Error> {
		let session = self.session.as_ref().ok_or(Error::Unavailable)?;
		// SAFETY: Running a new configuration on a live session keeps its
		// map and anchors.
		unsafe { session.runWithConfiguration(&Self::configuration(features)) };
		self.features = features;
		Ok(match config.reference_space {
			ReferenceSpace::Local => ReferenceSpace::Local,
			_ => ReferenceSpace::World,
		})
	}

	fn update(&mut self, _time: f64) -> Result<Option<XrFrame>, Error> {
		let session = self.session.clone().ok_or(Error::Unavailable)?;
		// SAFETY: Property reads.
		let Some(frame) = (unsafe { session.currentFrame() }) else {
			return Ok(None);
		};
		// SAFETY: As above.
		let time = unsafe { frame.timestamp() };

		#[allow(
			clippy::float_cmp,
			reason = "an unchanged frame repeats its exact timestamp"
		)]
		if time == self.last_time {
			return Ok(None);
		}

		self.last_time = time;
		Ok(Some(self.frame(&frame)))
	}

	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		let Some(session) = self.session.clone() else {
			return Vec::new();
		};
		let Some(ray) = hit_test::request_ray(request.origin, &frame.views) else {
			return Vec::new();
		};
		let mut hits = Vec::new();

		for (target, kind) in [
			(HitTargets::PLANES, RAYCAST_PLANE),
			(HitTargets::INFINITE_PLANES, RAYCAST_INFINITE),
			(HitTargets::ESTIMATED, RAYCAST_ESTIMATED),
		] {
			if request.targets.contains(target) {
				hits.extend(self.raycast(&session, ray, kind));
			}
		}

		if request.targets.contains(HitTargets::DEPTH)
			&& let (Some(depth), Some(view)) = (&frame.depth, frame.views.first())
		{
			hits.extend(hit_test::hit_depth(ray, view, depth));
		}

		hits
	}

	fn add_anchor(&mut self, pose: Pose, _plane: Option<u64>) -> Result<u64, Error> {
		let session = self.session.clone().ok_or(Error::Unavailable)?;
		let transform = simd::Float4x4::from_array(&pose.to_matrix());
		// SAFETY: Allocates and initialises an anchor, then adds it.
		let anchor = unsafe {
			let allocated: *mut AnyObject = objc2::msg_send![ARAnchor::class(), alloc];
			Retained::from_raw(simd::init_anchor(allocated, transform).cast::<ARAnchor>())
				.ok_or(Error::Native)?
		};
		// SAFETY: As above.
		unsafe { session.addAnchor(&anchor) };
		self.next_anchor += 1;
		self.anchors.insert(self.next_anchor, anchor);
		Ok(self.next_anchor)
	}

	fn remove_anchor(&mut self, id: u64) -> Result<(), Error> {
		let anchor = self.anchors.remove(&id).ok_or(Error::InvalidArgument)?;

		if let Some(session) = &self.session {
			// SAFETY: Removes an anchor this backend added.
			unsafe { session.removeAnchor(&anchor) };
		}

		Ok(())
	}

	fn set_viewport(&mut self, viewport: Viewport) {
		self.viewport = viewport;
	}

	fn stop(&mut self) {
		if let Some(session) = self.session.take() {
			// SAFETY: Pauses the session, which releases the camera.
			unsafe { session.pause() };
		}

		self.anchors.clear();
	}
}
