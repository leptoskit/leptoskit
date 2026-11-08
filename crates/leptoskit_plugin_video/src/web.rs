//! Browser Direct video: one HTMLMediaElement per player id, attached to
//! `Video` views that render `<video data-lk-video="id">`.

use std::cell::RefCell;
use std::collections::HashMap;

use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use web_sys::HtmlVideoElement;

use crate::CREATE;
use crate::PAUSE;
use crate::PLAY;
use crate::PlayerCommand;
use crate::PlayerFlag;
use crate::PlayerId;
use crate::RELEASE;
use crate::REPLACE;
use crate::Rate;
use crate::SEEK;
use crate::SET_LOOP;
use crate::SET_MUTED;
use crate::SET_RATE;
use crate::STATUS;
use crate::Seek;
use crate::Source;
use crate::Status;
use crate::TIME_UNKNOWN;

thread_local! {
	static PLAYERS: RefCell<HashMap<u32, Player>> = RefCell::new(HashMap::new());
	static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
}

struct Player {
	element: HtmlVideoElement,
	looping: bool,
	muted: bool,
	rate: f64,
}

/// Direct browser video API mirroring the native guest methods.
#[derive(Debug, Clone, Copy, Default)]
pub struct Video;

impl Video {
	/// Creates a detached `<video>` for `source` and returns its id.
	pub fn create(self, source: &Source) -> Result<PlayerId, Error> {
		let element = document_video()?;
		element.set_src(source.as_str());
		// web-sys has no playsInline setter; the attribute is the portable API.
		let _ = element.set_attribute("playsinline", "");
		let id = NEXT_ID.with(|next| {
			let id = *next.borrow();
			*next.borrow_mut() = id.saturating_add(1).max(1);
			id
		});
		PLAYERS.with(|players| {
			players.borrow_mut().insert(
				id,
				Player {
					element,
					looping: false,
					muted: false,
					rate: 1.0,
				},
			);
		});
		attach_surfaces();
		Ok(PlayerId(id))
	}

	/// Replaces the media URL on an existing player.
	pub fn replace(self, player: PlayerId, source: &Source) -> Result<(), Error> {
		with_player(player, |player| {
			player.element.set_src(source.as_str());
			Ok(())
		})
	}

	/// Starts playback; browsers may still require a user gesture.
	pub fn play(self, player: PlayerId) -> Result<(), Error> {
		with_player(player, |player| {
			let _ = player.element.play();
			Ok(())
		})
	}

	/// Pauses playback.
	pub fn pause(self, player: PlayerId) -> Result<(), Error> {
		with_player(player, |player| {
			let _ = player.element.pause();
			Ok(())
		})
	}

	/// Seeks to a millisecond offset.
	pub fn seek(self, player: PlayerId, position_ms: u32) -> Result<(), Error> {
		let seek = Seek::new(player, position_ms)?;
		with_player(seek.player, |player| {
			player
				.element
				.set_current_time(f64::from(seek.position_ms) / 1_000.0);
			Ok(())
		})
	}

	/// Mutes or unmutes.
	pub fn set_muted(self, player: PlayerId, muted: bool) -> Result<(), Error> {
		with_player(player, |player| {
			player.muted = muted;
			player.element.set_muted(muted);
			Ok(())
		})
	}

	/// Enables or disables looping.
	pub fn set_loop(self, player: PlayerId, looping: bool) -> Result<(), Error> {
		with_player(player, |player| {
			player.looping = looping;
			player.element.set_loop(looping);
			Ok(())
		})
	}

	/// Sets playback rate in thousandths.
	pub fn set_rate(self, player: PlayerId, thousandths: u32) -> Result<(), Error> {
		let rate = Rate::new(player, thousandths)?;
		with_player(rate.player, |player| {
			let value = f64::from(rate.thousandths) / 1_000.0;
			player.rate = value;
			player.element.set_playback_rate(value);
			Ok(())
		})
	}

	/// Snapshots playing / current / duration.
	pub fn status(self, player: PlayerId) -> Result<Status, Error> {
		with_player(player, |player| {
			let duration = player.element.duration();
			let current = player.element.current_time();
			Ok(Status {
				playing: !player.element.paused(),
				current_ms: seconds_to_ms(current),
				duration_ms: if duration.is_finite() {
					seconds_to_ms(duration)
				} else {
					TIME_UNKNOWN
				},
			})
		})
	}

	/// Drops the player and detaches any bound surface.
	pub fn release(self, player: PlayerId) -> Result<(), Error> {
		let removed = PLAYERS.with(|players| players.borrow_mut().remove(&player.0));
		if removed.is_none() {
			return Err(Error::InvalidArgument);
		}
		attach_surfaces();
		Ok(())
	}

	/// Dispatches a Direct method for hosts that speak the wire protocol.
	pub fn invoke(self, method: u32, payload: &[u8]) -> Result<leptoskit_bridge::Bytes, Error> {
		match method {
			x if x == CREATE.0 => {
				let source = Source::decode(payload)?;
				Ok(self.create(&source)?.encode())
			}
			x if x == REPLACE.0 => {
				let (player, source) = Source::decode_replace(payload)?;
				self.replace(player, &source)?;
				Ok(empty_bytes())
			}
			x if x == PLAY.0 => {
				self.play(PlayerCommand::decode(payload)?.player)?;
				Ok(empty_bytes())
			}
			x if x == PAUSE.0 => {
				self.pause(PlayerCommand::decode(payload)?.player)?;
				Ok(empty_bytes())
			}
			x if x == SEEK.0 => {
				let seek = Seek::decode(payload)?;
				self.seek(seek.player, seek.position_ms)?;
				Ok(empty_bytes())
			}
			x if x == SET_MUTED.0 => {
				let flag = PlayerFlag::decode(payload)?;
				self.set_muted(flag.player, flag.enabled)?;
				Ok(empty_bytes())
			}
			x if x == SET_LOOP.0 => {
				let flag = PlayerFlag::decode(payload)?;
				self.set_loop(flag.player, flag.enabled)?;
				Ok(empty_bytes())
			}
			x if x == SET_RATE.0 => {
				let rate = Rate::decode(payload)?;
				self.set_rate(rate.player, rate.thousandths)?;
				Ok(empty_bytes())
			}
			x if x == STATUS.0 => {
				let player = PlayerCommand::decode(payload)?.player;
				Ok(self.status(player)?.encode())
			}
			x if x == RELEASE.0 => {
				self.release(PlayerCommand::decode(payload)?.player)?;
				Ok(empty_bytes())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

fn empty_bytes() -> leptoskit_bridge::Bytes {
	leptoskit_bridge::codec::Encoder::with_capacity(0).finish()
}

fn with_player<T>(
	player: PlayerId,
	body: impl FnOnce(&mut Player) -> Result<T, Error>,
) -> Result<T, Error> {
	PLAYERS.with(|players| {
		let mut players = players.borrow_mut();
		let player = players.get_mut(&player.0).ok_or(Error::InvalidArgument)?;
		body(player)
	})
}

fn document_video() -> Result<HtmlVideoElement, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let document = window.document().ok_or(Error::Unavailable)?;
	let element = document
		.create_element("video")
		.map_err(|_| Error::Native)?;
	element
		.dyn_into::<HtmlVideoElement>()
		.map_err(|_| Error::Native)
}

fn seconds_to_ms(seconds: f64) -> u32 {
	if !seconds.is_finite() || seconds < 0.0 {
		return TIME_UNKNOWN;
	}
	let ms = (seconds * 1_000.0).round();
	if ms > f64::from(u32::MAX - 1) {
		TIME_UNKNOWN
	} else {
		ms as u32
	}
}

/// Copies each player's src/muted/loop/rate onto matching `data-lk-video`
/// nodes.
fn attach_surfaces() {
	let Some(window) = web_sys::window() else {
		return;
	};
	let Some(document) = window.document() else {
		return;
	};
	PLAYERS.with(|players| {
		let players = players.borrow();
		for (id, player) in players.iter() {
			let selector = format!("video[data-lk-video=\"{id}\"]");
			let Ok(nodes) = document.query_selector_all(&selector) else {
				continue;
			};
			for index in 0..nodes.length() {
				let Some(node) = nodes.item(index) else {
					continue;
				};
				let Ok(surface) = node.dyn_into::<HtmlVideoElement>() else {
					continue;
				};
				if surface.src() != player.element.src() {
					surface.set_src(&player.element.src());
				}
				surface.set_muted(player.muted);
				surface.set_loop(player.looping);
				surface.set_playback_rate(player.rate);
				let _ = surface.set_attribute("playsinline", "");
			}
		}
	});
}

#[cfg(test)]
mod tests {
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	use crate::RATE_NORMAL;

	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen_test]
	fn create_controls_status_and_release() {
		let video = Video;
		let source = Source::url("https://example.com/clip.mp4").unwrap();
		let id = video.create(&source).unwrap();
		video
			.replace(id, &Source::url("https://example.com/other.mp4").unwrap())
			.unwrap();
		video.set_muted(id, true).unwrap();
		video.set_loop(id, true).unwrap();
		video.set_rate(id, 1_250).unwrap();
		video.seek(id, 250).unwrap();
		let _ = video.play(id);
		video.pause(id).unwrap();
		let status = video.status(id).unwrap();
		assert!(!status.playing);
		assert_eq!(
			video.release(PlayerId(id.0.wrapping_add(99))),
			Err(Error::InvalidArgument)
		);
		video.release(id).unwrap();
		assert_eq!(video.status(id), Err(Error::InvalidArgument));
	}

	#[wasm_bindgen_test]
	fn invoke_round_trips_and_rejects_unknown() {
		let video = Video;
		let create = Source::url("https://example.com/a.mp4")
			.unwrap()
			.encode()
			.unwrap();
		let id_bytes = video.invoke(CREATE.0, &create).unwrap();
		let id = PlayerId::decode(&id_bytes).unwrap();
		let replace =
			Source::encode_replace(id, &Source::url("https://example.com/b.mp4").unwrap()).unwrap();
		assert!(video.invoke(REPLACE.0, &replace).is_ok());
		assert!(
			video
				.invoke(PLAY.0, &PlayerCommand { player: id }.encode())
				.is_ok()
		);
		assert!(
			video
				.invoke(PAUSE.0, &PlayerCommand { player: id }.encode())
				.is_ok()
		);
		assert!(
			video
				.invoke(SEEK.0, &Seek::new(id, 10).unwrap().encode())
				.is_ok()
		);
		assert!(
			video
				.invoke(
					SET_MUTED.0,
					&PlayerFlag {
						player: id,
						enabled: false
					}
					.encode()
				)
				.is_ok()
		);
		assert!(
			video
				.invoke(
					SET_LOOP.0,
					&PlayerFlag {
						player: id,
						enabled: false
					}
					.encode()
				)
				.is_ok()
		);
		assert!(
			video
				.invoke(SET_RATE.0, &Rate::new(id, RATE_NORMAL).unwrap().encode())
				.is_ok()
		);
		assert!(
			video
				.invoke(STATUS.0, &PlayerCommand { player: id }.encode())
				.is_ok()
		);
		assert_eq!(video.invoke(99, &[]), Err(Error::Unavailable));
		assert!(
			video
				.invoke(RELEASE.0, &PlayerCommand { player: id }.encode())
				.is_ok()
		);
	}

	#[test]
	fn seconds_to_ms_bounds() {
		assert_eq!(seconds_to_ms(f64::NAN), TIME_UNKNOWN);
		assert_eq!(seconds_to_ms(-1.0), TIME_UNKNOWN);
		assert_eq!(seconds_to_ms(1.5), 1_500);
		assert_eq!(seconds_to_ms(f64::from(u32::MAX)), TIME_UNKNOWN);
	}
}
