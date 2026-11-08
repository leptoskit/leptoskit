//! Decoding with symphonia: whole sounds into memory, or a second ahead of a
//! streaming voice on a decoder thread.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use leptoskit_bridge::Error;
use rtrb::Producer;
use rtrb::RingBuffer;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoder;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::FormatReader;
use symphonia::core::formats::TrackType;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSource;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::io::MediaSourceStreamOptions;
use symphonia::core::meta::MetadataOptions;

use super::mixer::Feed;

/// The most samples a sound decoded into memory may hold: 64 MiB, a little
/// under three minutes of stereo at 48 kHz. Longer sounds must stream.
pub const MAX_MEMORY_SAMPLES: usize = 16 * 1024 * 1024;
/// Encoded files larger than this stream in [`crate::LoadMode::Auto`].
pub const STREAM_THRESHOLD: u64 = 512 * 1024;
/// How far ahead of a streaming voice its decoder keeps.
const STREAM_SECONDS: usize = 1;

/// A decoded sound: interleaved stereo at its own rate.
#[derive(Debug)]
pub struct Pcm {
	rate: u32,
	samples: Box<[f32]>,
}

impl Pcm {
	#[cfg(test)]
	pub fn from_mono(rate: u32, mono: &[f32]) -> Self {
		Self {
			rate,
			samples: mono.iter().flat_map(|&sample| [sample, sample]).collect(),
		}
	}

	#[cfg(test)]
	pub fn left(&self) -> impl Iterator<Item = &f32> {
		self.samples.iter().step_by(2)
	}

	pub fn rate(&self) -> u32 {
		self.rate
	}

	pub fn frames(&self) -> usize {
		self.samples.len() / 2
	}

	#[inline]
	pub fn frame(&self, frame: usize) -> [f32; 2] {
		[self.samples[frame * 2], self.samples[frame * 2 + 1]]
	}
}

/// Where encoded bytes are read from.
#[derive(Debug, Clone)]
pub enum Encoded {
	File(PathBuf),
	Bytes(Arc<[u8]>),
}

impl Encoded {
	pub fn len(&self) -> Result<u64, Error> {
		match self {
			Self::File(path) => {
				std::fs::metadata(path)
					.map(|metadata| metadata.len())
					.map_err(|error| io_error(&error))
			}
			Self::Bytes(bytes) => Ok(bytes.len() as u64),
		}
	}

	fn extension(&self) -> Option<&str> {
		match self {
			Self::File(path) => path.extension()?.to_str(),
			Self::Bytes(_) => None,
		}
	}

	fn open(&self) -> Result<Box<dyn MediaSource>, Error> {
		Ok(match self {
			Self::File(path) => {
				Box::new(std::fs::File::open(path).map_err(|error| io_error(&error))?)
			}
			Self::Bytes(bytes) => Box::new(Cursor::new(Arc::clone(bytes))),
		})
	}
}

fn io_error(error: &std::io::Error) -> Error {
	match error.kind() {
		std::io::ErrorKind::NotFound => Error::Unavailable,
		std::io::ErrorKind::PermissionDenied => Error::PermissionDenied,
		_ => Error::Native,
	}
}

fn decode_error(error: &DecodeError) -> Error {
	match error {
		DecodeError::IoError(error) if error.kind() == std::io::ErrorKind::NotFound => {
			Error::Unavailable
		}

		DecodeError::Unsupported(_) | DecodeError::DecodeError(_) | DecodeError::IoError(_) => {
			Error::InvalidArgument
		}
		_ => Error::Native,
	}
}

/// What a sound is, as its container describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
	pub rate: u32,
	pub channels: u32,
	pub frames: Option<u64>,
}

/// One open, decoding sound.
pub struct Decoder {
	reader: Box<dyn FormatReader>,
	codec: Box<dyn AudioDecoder>,
	track: u32,
	shape: Shape,
	scratch: Vec<f32>,
}

impl std::fmt::Debug for Decoder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Decoder")
			.field("shape", &self.shape)
			.finish_non_exhaustive()
	}
}

impl Decoder {
	pub fn open(encoded: &Encoded) -> Result<Self, Error> {
		let stream = MediaSourceStream::new(encoded.open()?, MediaSourceStreamOptions::default());
		let mut hint = Hint::new();

		if let Some(extension) = encoded.extension() {
			hint.with_extension(extension);
		}

		let reader = symphonia::default::get_probe()
			.probe(
				&hint,
				stream,
				FormatOptions::default(),
				MetadataOptions::default(),
			)
			.map_err(|error| decode_error(&error))?;
		let track = reader
			.default_track(TrackType::Audio)
			.ok_or(Error::InvalidArgument)?;
		let Some(CodecParameters::Audio(parameters)) = &track.codec_params else {
			return Err(Error::InvalidArgument);
		};

		let codec = symphonia::default::get_codecs()
			.make_audio_decoder(parameters, &AudioDecoderOptions::default())
			.map_err(|error| decode_error(&error))?;
		let shape = Shape {
			rate: parameters.sample_rate.ok_or(Error::InvalidArgument)?,
			channels: parameters
				.channels
				.as_ref()
				.map_or(0, |channels| channels.count() as u32),
			frames: track.num_frames,
		};

		if shape.rate == 0 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			track: track.id,
			reader,
			codec,
			shape,
			scratch: Vec::new(),
		})
	}

	pub fn shape(&self) -> Shape {
		self.shape
	}

	/// Appends the next packet's frames to `output` as interleaved stereo.
	/// Returns false at the end of the sound.
	pub fn decode_into(&mut self, output: &mut Vec<f32>) -> Result<bool, Error> {
		loop {
			let Some(packet) = self
				.reader
				.next_packet()
				.map_err(|error| decode_error(&error))?
			else {
				return Ok(false);
			};

			if packet.track_id != self.track {
				continue;
			}

			let decoded = match self.codec.decode(&packet) {
				Ok(decoded) => decoded,
				// A corrupt packet is skipped, as players do.
				Err(DecodeError::DecodeError(_)) => continue,
				Err(error) => return Err(decode_error(&error)),
			};

			let channels = decoded.spec().channels().count();

			if channels == 0 {
				return Err(Error::InvalidArgument);
			}

			decoded.copy_to_vec_interleaved(&mut self.scratch);
			stereo(&self.scratch, channels, output);

			return Ok(true);
		}
	}

	/// Decodes the whole sound.
	pub fn decode_all(mut self) -> Result<Pcm, Error> {
		let capacity = self
			.shape
			.frames
			.map_or(0, |frames| (frames as usize).saturating_mul(2))
			.min(MAX_MEMORY_SAMPLES);
		let mut samples = Vec::with_capacity(capacity);

		while self.decode_into(&mut samples)? {
			if samples.len() > MAX_MEMORY_SAMPLES {
				return Err(Error::TooLarge);
			}
		}

		Ok(Pcm {
			rate: self.shape.rate,
			samples: samples.into_boxed_slice(),
		})
	}
}

/// Appends interleaved frames of any channel count as stereo: mono is
/// doubled, and channels past the second are left out.
fn stereo(samples: &[f32], channels: usize, output: &mut Vec<f32>) {
	output.reserve(samples.len() / channels * 2);

	for frame in samples.chunks_exact(channels) {
		let left = frame[0];
		let right = if channels == 1 { left } else { frame[1] };
		output.push(left);
		output.push(right);
	}
}

/// Decodes one streaming voice ahead of the mixer.
struct Job {
	encoded: Encoded,
	decoder: Decoder,
	frames: Producer<f32>,
	looping: bool,
	finished: Arc<AtomicBool>,
	pending: Vec<f32>,
	offset: usize,
}

impl Job {
	/// Decodes until the voice's queue is full. Returns false when the job is
	/// over: the sound ended, failed, or its voice is gone.
	fn fill(&mut self) -> bool {
		if self.frames.is_abandoned() {
			return false;
		}

		let mut restarted = false;

		while self.flush() {
			match self.decoder.decode_into(&mut self.pending) {
				Ok(true) => restarted = false,
				// Looping starts a fresh decoder: the next frame after the last
				// is the first, with the codec's own padding trimmed. A sound
				// that ends again at once has nothing in it.
				Ok(false) if self.looping && !restarted => {
					let Ok(decoder) = Decoder::open(&self.encoded) else {
						return self.finish();
					};
					self.decoder = decoder;
					restarted = true;
				}
				Ok(false) | Err(_) => return self.finish(),
			}
		}

		true
	}

	/// Moves decoded frames into the voice's queue. Returns true once they
	/// are all there and the buffer is empty for the next packet; false while
	/// the queue is full.
	fn flush(&mut self) -> bool {
		let waiting = self.pending.len() - self.offset;
		// Whole frames only, so the mixer never reads half of one.
		let count = self.frames.slots().min(waiting) & !1;

		if let Ok(mut chunk) = self.frames.write_chunk(count) {
			let (first, second) = chunk.as_mut_slices();
			let source = &self.pending[self.offset..self.offset + count];
			first.copy_from_slice(&source[..first.len()]);
			second.copy_from_slice(&source[first.len()..]);
			chunk.commit_all();
			self.offset += count;
		}

		if self.offset < self.pending.len() {
			return false;
		}

		self.pending.clear();
		self.offset = 0;
		true
	}

	fn finish(&self) -> bool {
		self.finished.store(true, Ordering::Release);
		false
	}
}

/// The thread that decodes every streaming voice.
pub struct Streamer {
	jobs: mpsc::Sender<Job>,
}

impl Streamer {
	pub fn spawn() -> Result<Self, Error> {
		let (jobs, incoming) = mpsc::channel::<Job>();
		std::thread::Builder::new()
			.name("leptoskit-audio-decoder".into())
			.spawn(move || run(&incoming))
			.map_err(|_| Error::Native)?;
		Ok(Self { jobs })
	}

	/// Opens `encoded`, fills the voice's queue so it starts without a gap,
	/// and hands the rest to this thread.
	pub fn feed(&self, encoded: &Encoded, looping: bool) -> Result<(Feed, Shape), Error> {
		let decoder = Decoder::open(encoded)?;
		let shape = decoder.shape();
		let capacity = shape.rate as usize * 2 * STREAM_SECONDS;
		let (frames, consumer) = RingBuffer::new(capacity);
		let finished = Arc::new(AtomicBool::new(false));
		let mut job = Job {
			encoded: encoded.clone(),
			decoder,
			frames,
			looping,
			finished: Arc::clone(&finished),
			pending: Vec::new(),
			offset: 0,
		};

		if job.fill() {
			self.jobs.send(job).map_err(|_| Error::Native)?;
		}

		Ok((
			Feed::Stream {
				frames: consumer,
				finished,
			},
			shape,
		))
	}
}

fn run(incoming: &mpsc::Receiver<Job>) {
	let mut jobs: Vec<Job> = Vec::new();

	loop {
		match incoming.recv_timeout(Duration::from_millis(10)) {
			Ok(job) => jobs.push(job),
			Err(mpsc::RecvTimeoutError::Timeout) => {}
			Err(mpsc::RecvTimeoutError::Disconnected) => return,
		}

		while let Ok(job) = incoming.try_recv() {
			jobs.push(job);
		}

		jobs.retain_mut(Job::fill);
	}
}

#[cfg(test)]
mod tests;
