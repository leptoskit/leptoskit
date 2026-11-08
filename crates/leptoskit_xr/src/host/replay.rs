//! Recorded sessions, played back exactly: for tests, and for simulators and
//! desktops that have no camera to track.
//!
//! Playback reads no clock of its own. The frame shown at a time `t` is the
//! last one recorded at or before `t` minus the time playback started, so the
//! same calls give the same frames on every run.

use std::path::Path;
use std::sync::Arc;

use leptoskit_bridge::Error;

use super::Backend;
use super::XrFrame;
use crate::image::DepthFormat;
use crate::image::DepthImage;
use crate::image::ImagePlane;
use crate::types::*;
use crate::wire::Reader;
use crate::wire::Wire;
use crate::wire::Writer;

/// The first bytes of a recording file.
const MAGIC: [u8; 4] = *b"LKXR";
/// The recording format this crate writes and reads.
const VERSION: u32 = 1;
/// The largest depth picture a recording holds: 1024 × 1024 four-byte
/// pixels.
const MAX_DEPTH_BYTES: usize = 4 << 20;
/// The most frames a recording holds: ten minutes at 60 frames a second.
const MAX_FRAMES: usize = 36_000;
/// Playback shows a frame this early, so that rounding in the recorded
/// times never delays one by a whole update.
const TIME_TOLERANCE: f64 = 1e-6;

/// One recorded frame: what the guest saw, plus the depth picture for hit
/// tests. Camera pictures are not recorded.
#[derive(Debug, Clone)]
pub struct RecordedFrame {
	/// The frame. Its `sequence` is ignored on playback.
	pub frame: Frame,
	/// The depth picture, when the session had one.
	pub depth: Option<DepthImage>,
}

/// A recorded session.
#[derive(Debug, Clone)]
pub struct Recording {
	/// What the recorded device could do; playback offers the same.
	pub capabilities: Capabilities,
	/// The frames, in time order.
	pub frames: Vec<RecordedFrame>,
}

impl Recording {
	/// How long the recording lasts, in seconds.
	#[must_use]
	pub fn duration(&self) -> f64 {
		match (self.frames.first(), self.frames.last()) {
			(Some(first), Some(last)) => last.frame.time - first.frame.time,
			_ => 0.0,
		}
	}

	/// The recording as bytes, to save.
	#[must_use]
	pub fn to_bytes(&self) -> Vec<u8> {
		let mut writer = Writer::new();
		writer.u32(u32::from_le_bytes(MAGIC));
		writer.u32(VERSION);
		self.capabilities.write(&mut writer);
		writer.count(self.frames.len());

		for recorded in &self.frames {
			recorded.frame.write(&mut writer);
			writer.u32(u32::from(recorded.depth.is_some()));

			if let Some(depth) = &recorded.depth {
				write_depth(&mut writer, depth);
			}
		}

		writer.finish()
	}

	/// Reads a recording, refusing anything malformed, out of order, or too
	/// large.
	pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(bytes);

		if reader.u32()? != u32::from_le_bytes(MAGIC) || reader.u32()? != VERSION {
			return Err(Error::InvalidArgument);
		}

		let capabilities = Capabilities::read(&mut reader)?;
		let count = reader.count(MAX_FRAMES)?;
		let mut frames: Vec<RecordedFrame> = Vec::with_capacity(count);

		for _ in 0..count {
			let frame = Frame::read(&mut reader)?;
			let depth = if reader.bool()? {
				Some(read_depth(&mut reader)?)
			} else {
				None
			};

			if frames
				.last()
				.is_some_and(|last| last.frame.time > frame.time)
			{
				return Err(Error::InvalidArgument);
			}

			frames.push(RecordedFrame { frame, depth });
		}

		reader.finish()?;
		Ok(Self {
			capabilities,
			frames,
		})
	}

	/// Reads a recording file.
	pub fn open(path: &Path) -> std::io::Result<Self> {
		let bytes = std::fs::read(path)?;
		Self::from_bytes(&bytes).map_err(|_| {
			std::io::Error::new(
				std::io::ErrorKind::InvalidData,
				format!("{} is not a leptoskit XR recording", path.display()),
			)
		})
	}
}

fn depth_format(value: u32) -> Result<DepthFormat, Error> {
	match value {
		0 => Ok(DepthFormat::Meters32),
		1 => Ok(DepthFormat::Millimeters16),
		_ => Err(Error::InvalidArgument),
	}
}

fn write_depth(writer: &mut Writer, depth: &DepthImage) {
	writer.u32(depth.width);
	writer.u32(depth.height);
	writer.u32(match depth.format {
		DepthFormat::Meters32 => 0,
		DepthFormat::Millimeters16 => 1,
	});
	writer.u32(depth.row_stride);
	writer.bytes(&depth.data);

	for value in depth.display_transform {
		writer.f32(value);
	}

	writer.u32(u32::from(depth.confidence.is_some()));

	if let Some(confidence) = &depth.confidence {
		writer.u32(confidence.width);
		writer.u32(confidence.height);
		writer.u32(confidence.row_stride);
		writer.bytes(&confidence.data);
	}
}

fn read_depth(reader: &mut Reader<'_>) -> Result<DepthImage, Error> {
	let width = reader.u32()?;
	let height = reader.u32()?;
	let format = depth_format(reader.u32()?)?;
	let row_stride = reader.u32()?;
	let data: Arc<[u8]> = reader.bytes(MAX_DEPTH_BYTES)?.into();
	let mut display_transform = [0.0; 6];

	for value in &mut display_transform {
		*value = reader.f32()?;
	}

	let confidence = if reader.bool()? {
		Some(ImagePlane {
			width: reader.u32()?,
			height: reader.u32()?,
			row_stride: reader.u32()?,
			data: reader.bytes(MAX_DEPTH_BYTES)?.into(),
		})
	} else {
		None
	};
	let plane = ImagePlane {
		width,
		height,
		row_stride,
		data: Arc::clone(&data),
	};

	if !plane.is_complete(format.bytes()) || confidence.as_ref().is_some_and(|c| !c.is_complete(1))
	{
		return Err(Error::InvalidArgument);
	}

	Ok(DepthImage {
		width,
		height,
		format,
		data,
		row_stride,
		confidence,
		display_transform,
	})
}

/// Collects a live session's frames into a [`Recording`].
#[derive(Debug)]
pub struct Recorder {
	recording: Recording,
}

impl Recorder {
	/// An empty recording of a device with `capabilities`.
	#[must_use]
	pub fn new(capabilities: Capabilities) -> Self {
		Self {
			recording: Recording {
				capabilities,
				frames: Vec::new(),
			},
		}
	}

	/// Adds a frame. Frames older than the last one, and frames past the
	/// recording's limit, are skipped.
	pub fn record(&mut self, frame: &XrFrame) {
		let frames = &mut self.recording.frames;

		if frames.len() >= MAX_FRAMES
			|| frames
				.last()
				.is_some_and(|last| last.frame.time > frame.time)
		{
			return;
		}

		frames.push(RecordedFrame {
			frame: frame.to_guest(0, true),
			depth: frame.depth.clone(),
		});
	}

	/// The finished recording.
	#[must_use]
	pub fn finish(self) -> Recording {
		self.recording
	}
}

/// Plays a [`Recording`] back as if it were a live device.
#[derive(Debug)]
pub struct ReplayBackend {
	recording: Arc<Recording>,
	looping: bool,
	running: bool,
	started: Option<f64>,
	shown: Option<(u64, usize)>,
}

impl ReplayBackend {
	/// A backend that plays `recording` once and then holds its last frame.
	#[must_use]
	pub fn new(recording: Recording) -> Self {
		Self {
			recording: Arc::new(recording),
			looping: false,
			running: false,
			started: None,
			shown: None,
		}
	}

	/// Plays the recording over and over.
	#[must_use]
	pub fn looping(mut self, looping: bool) -> Self {
		self.looping = looping;
		self
	}

	/// The frame to show `elapsed` seconds into playback: which pass through
	/// the recording, and which frame of it.
	fn position(&self, elapsed: f64) -> Option<(u64, usize)> {
		let frames = &self.recording.frames;
		let first = frames.first()?.frame.time;
		let duration = self.recording.duration();
		let (pass, offset) = if self.looping && duration > 0.0 {
			let passes = (elapsed / duration).floor();
			(passes as u64, elapsed - passes * duration)
		} else {
			(0, elapsed)
		};
		let index = frames
			.partition_point(|recorded| recorded.frame.time - first <= offset + TIME_TOLERANCE)
			.checked_sub(1)?;

		Some((pass, index))
	}

	fn frame(&self, pass: u64, index: usize) -> XrFrame {
		let recorded = &self.recording.frames[index];
		let frame = &recorded.frame;

		XrFrame {
			time: frame.time + pass as f64 * self.recording.duration(),
			tracking: frame.tracking,
			viewer: frame.viewer,
			views: frame.views.clone(),
			camera_image: None,
			depth: recorded.depth.clone(),
			planes: frame.planes.clone().into(),
			planes_revision: 0,
			light: frame.light,
			anchors: frame.anchors.clone(),
			inputs: frame.inputs.clone(),
		}
	}
}

impl Backend for ReplayBackend {
	fn capabilities(&self) -> Capabilities {
		self.recording.capabilities
	}

	fn prepare(&mut self, mode: Mode, _install: bool) -> Availability {
		if self.recording.capabilities.supports(mode) {
			Availability::Ready
		} else {
			Availability::Unsupported
		}
	}

	fn start(
		&mut self,
		config: &SessionConfig,
		_features: Features,
	) -> Result<ReferenceSpace, Error> {
		self.running = true;
		self.started = None;
		self.shown = None;
		Ok(config.reference_space)
	}

	fn configure(
		&mut self,
		config: &SessionConfig,
		_features: Features,
	) -> Result<ReferenceSpace, Error> {
		Ok(config.reference_space)
	}

	fn update(&mut self, time: f64) -> Result<Option<XrFrame>, Error> {
		if !self.running {
			return Err(Error::Unavailable);
		}

		let started = *self.started.get_or_insert(time);
		let Some(position) = self.position((time - started).max(0.0)) else {
			return Ok(None);
		};

		if self.shown == Some(position) {
			return Ok(None);
		}

		self.shown = Some(position);
		Ok(Some(self.frame(position.0, position.1)))
	}

	fn stop(&mut self) {
		self.running = false;
	}
}
