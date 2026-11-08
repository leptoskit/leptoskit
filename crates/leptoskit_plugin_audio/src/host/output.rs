//! The device output: a thread that owns the cpal stream, whose callback runs
//! the mixer. cpal follows the default device where the backend can; when the
//! stream is lost anyway, the engine opens a new one.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use cpal::FromSample;
use cpal::SampleFormat;
use cpal::SizedSample;
use cpal::StreamConfig;
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use leptoskit_bridge::Error;

use super::mixer::Handle;
use super::mixer::Mixer;

/// The largest callback converted at once when the device takes integers.
const SCRATCH_FRAMES: usize = 4_096;

enum Control {
	/// Stop the device once a hold's fade has finished.
	Hold(Duration),
	Resume,
}

/// A running output and the thread that owns it.
pub struct Output {
	control: mpsc::Sender<Control>,
	lost: Arc<AtomicBool>,
	pub rate: u32,
}

impl Output {
	/// Opens the default output device and starts the mixer on it.
	pub fn open() -> Result<(Self, Handle), Error> {
		let (control, requests) = mpsc::channel();
		let (opened, result) = mpsc::sync_channel(1);
		let lost = Arc::new(AtomicBool::new(false));
		let flag = Arc::clone(&lost);

		std::thread::Builder::new()
			.name("leptoskit-audio-output".into())
			.spawn(move || serve(flag, &opened, &requests))
			.map_err(|_| Error::Native)?;

		let (handle, rate) = result.recv().map_err(|_| Error::Native)??;
		Ok((
			Self {
				control,
				lost,
				rate,
			},
			handle,
		))
	}

	/// Whether the device went away and a new output is needed.
	pub fn lost(&self) -> bool {
		self.lost.load(Ordering::Acquire)
	}

	/// Lets the device sleep once `fade` has silenced it.
	pub fn hold(&self, fade: Duration) {
		let _ = self.control.send(Control::Hold(fade));
	}

	pub fn resume(&self) {
		let _ = self.control.send(Control::Resume);
	}
}

/// Opens the stream on this thread, reports how that went, and keeps it
/// until the engine lets go.
fn serve(
	lost: Arc<AtomicBool>,
	opened: &mpsc::SyncSender<Result<(Handle, u32), Error>>,
	requests: &mpsc::Receiver<Control>,
) {
	match start(lost) {
		Ok((stream, handle, rate)) => {
			if opened.send(Ok((handle, rate))).is_ok() {
				run(&stream, requests);
			}
		}
		Err(error) => {
			let _ = opened.send(Err(error));
		}
	}
}

/// Owns the stream until the engine drops its [`Output`].
fn run(stream: &cpal::Stream, requests: &mpsc::Receiver<Control>) {
	let mut sleep_at: Option<Instant> = None;

	loop {
		let request = match sleep_at {
			Some(at) => requests.recv_timeout(at.saturating_duration_since(Instant::now())),
			None => {
				requests
					.recv()
					.map_err(|_| mpsc::RecvTimeoutError::Disconnected)
			}
		};

		match request {
			// A little past the fade, so its last frames reach the speaker.
			Ok(Control::Hold(fade)) => {
				sleep_at = Some(Instant::now() + fade + Duration::from_millis(50));
			}
			Ok(Control::Resume) => {
				sleep_at = None;
				let _ = stream.play();
			}
			Err(mpsc::RecvTimeoutError::Timeout) => {
				sleep_at = None;
				let _ = stream.pause();
			}
			Err(mpsc::RecvTimeoutError::Disconnected) => return,
		}
	}
}

fn start(lost: Arc<AtomicBool>) -> Result<(cpal::Stream, Handle, u32), Error> {
	let host = cpal::default_host();
	let device = host.default_output_device().ok_or(Error::Unavailable)?;
	let (config, format) = configuration(&device)?;
	let (mixer, handle) = Mixer::new(config.sample_rate);
	let rate = config.sample_rate;

	let stream = match format {
		SampleFormat::F32 => build_f32(&device, &config, mixer, lost),
		SampleFormat::I16 => build::<i16>(&device, &config, mixer, lost),
		SampleFormat::U16 => build::<u16>(&device, &config, mixer, lost),
		SampleFormat::I32 => build::<i32>(&device, &config, mixer, lost),
		_ => Err(Error::Unavailable),
	}?;

	stream.play().map_err(|_| Error::Native)?;
	Ok((stream, handle, rate))
}

/// The device's own configuration. Android's `AAudio` is asked for stereo at
/// 48 kHz directly: reading its device list needs a Java context that the
/// stream itself does not.
fn configuration(device: &cpal::Device) -> Result<(StreamConfig, SampleFormat), Error> {
	if cfg!(target_os = "android") {
		return Ok((
			StreamConfig {
				channels: 2,
				sample_rate: 48_000,
				buffer_size: cpal::BufferSize::Default,
			},
			SampleFormat::F32,
		));
	}

	let supported = device
		.default_output_config()
		.map_err(|_| Error::Unavailable)?;
	Ok((supported.config(), supported.sample_format()))
}

fn on_error(lost: Arc<AtomicBool>) -> impl FnMut(cpal::Error) + Send + 'static {
	move |error| {
		if matches!(
			error.kind(),
			cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated
		) {
			lost.store(true, Ordering::Release);
		}
	}
}

fn build_f32(
	device: &cpal::Device,
	config: &StreamConfig,
	mut mixer: Mixer,
	lost: Arc<AtomicBool>,
) -> Result<cpal::Stream, Error> {
	let channels = usize::from(config.channels);
	device
		.build_output_stream(
			*config,
			move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
				mixer.render(output, channels);
			},
			on_error(lost),
			None,
		)
		.map_err(|_| Error::Unavailable)
}

fn build<T>(
	device: &cpal::Device,
	config: &StreamConfig,
	mut mixer: Mixer,
	lost: Arc<AtomicBool>,
) -> Result<cpal::Stream, Error>
where
	T: SizedSample + FromSample<f32>,
{
	let channels = usize::from(config.channels);
	let mut scratch = vec![0.0_f32; SCRATCH_FRAMES * channels];
	device
		.build_output_stream(
			*config,
			move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
				for chunk in output.chunks_mut(scratch.len()) {
					let scratch = &mut scratch[..chunk.len()];
					mixer.render(scratch, channels);

					for (sample, mixed) in chunk.iter_mut().zip(scratch.iter()) {
						*sample = T::from_sample(*mixed);
					}
				}
			},
			on_error(lost),
			None,
		)
		.map_err(|_| Error::Unavailable)
}
