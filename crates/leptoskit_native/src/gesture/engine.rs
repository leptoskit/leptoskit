//! The arena for one surface: a track per pointer, recognizers collected from
//! the hit path, and the effects to deliver after the borrow ends.

use std::sync::Arc;

use leptoskit_yoga::NodeId;

use super::Axis;
use super::Calls;
use super::Gesture;
use super::GestureEvent;
use super::GestureNode;
use super::GestureState;
use super::GroupMode;
use super::PointerDecision;
use super::PointerKind;
use super::PointerPhase;
use super::PointerRoute;
use super::PointerSample;

pub(crate) const TOUCH_SLOP: f32 = 18.0;
pub(crate) const LONG_PRESS_MS: u64 = 500;
pub(crate) const FLING_SPEED: f32 = 800.0;
const MAX_TRACKS: usize = 16;
const VELOCITY_WINDOW_MS: u64 = 100;

type Callback = Arc<dyn Fn(GestureEvent) + Send + Sync>;

pub(crate) struct Effect {
	pub callback: Callback,
	pub event: GestureEvent,
}

#[derive(Default)]
pub(crate) struct Session {
	tracks: Vec<Track>,
	spans: Vec<Span>,
	effects: Vec<Effect>,
	pool: Vec<Effect>,
}

struct Track {
	id: u64,
	origin_x: f32,
	origin_y: f32,
	x: f32,
	y: f32,
	down_ms: u64,
	last_ms: u64,
	kind: PointerKind,
	members: Vec<Member>,
	samples: [(u64, f32, f32); 8],
	sample_at: u8,
	sample_len: u8,
	claimed: bool,
}

struct Member {
	node: NodeId,
	leaf: Leaf,
	calls: Calls,
	team: u16,
	gate: u16,
	priority: u16,
	contest: u16,
	phase: Phase,
}

struct LeafSpec {
	leaf: Leaf,
	calls: Calls,
}

#[derive(Clone)]
enum Leaf {
	Tap {
		slop: f32,
		max_ms: Option<u64>,
	},
	LongPress {
		slop: f32,
		duration_ms: u64,
	},
	Pan {
		slop: f32,
		axis: Axis,
		preempt: bool,
	},
	Pinch,
	Rotation,
	Fling {
		min_speed: f32,
	},
	Native {
		drag: bool,
	},
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
	Possible,
	Active,
	Failed,
	Done,
}

enum Intent {
	Wait,
	Accept,
	Reject,
}

struct Span {
	node: NodeId,
	primary: u64,
	points: [(u64, f32, f32); 2],
	count: u8,
	base_len: f32,
	angle: f32,
	rotation: f32,
	scale: f32,
	focal_x: f32,
	focal_y: f32,
}

pub(crate) struct Collected {
	pub node: NodeId,
	pub gesture: Option<Gesture>,
	pub native: Option<bool>,
}

impl Session {
	pub(crate) fn handle(
		&mut self,
		sample: PointerSample,
		collected: Vec<Collected>,
	) -> PointerDecision {
		if !sample.x.is_finite() || !sample.y.is_finite() {
			return self.decide(sample.time_ms);
		}

		match sample.phase {
			PointerPhase::Down => self.down(sample, collected),
			PointerPhase::Move => self.relocate(sample, false),
			PointerPhase::Up => self.relocate(sample, true),
			PointerPhase::Cancel => self.cancel_pointer(sample.id, sample.time_ms),
		}

		self.decide(sample.time_ms)
	}

	pub(crate) fn advance(&mut self, now: u64) -> PointerDecision {
		let ids: Vec<u64> = self.tracks.iter().map(|track| track.id).collect();

		for id in ids {
			self.resolve(id, PointerPhase::Down, now);
		}

		self.decide(now)
	}

	pub(crate) fn cancel_node(&mut self, node: NodeId, now: u64) {
		let ids: Vec<u64> = self
			.tracks
			.iter()
			.filter(|track| track.members.iter().any(|member| member.node == node))
			.map(|track| track.id)
			.collect();

		for id in ids {
			self.cancel_pointer(id, now);
		}
	}

	pub(crate) fn take_effects(&mut self) -> Vec<Effect> {
		std::mem::take(&mut self.effects)
	}

	pub(crate) fn recycle(&mut self, mut effects: Vec<Effect>) {
		effects.clear();

		if effects.capacity() > self.pool.capacity() {
			self.pool = effects;
		}
	}

	fn down(&mut self, sample: PointerSample, collected: Vec<Collected>) {
		if self.tracks.iter().any(|track| track.id == sample.id) {
			self.cancel_pointer(sample.id, sample.time_ms);
		}

		if self.tracks.len() >= MAX_TRACKS {
			return;
		}

		let members = members_from(collected);

		if members.is_empty() {
			return;
		}

		let mut track = Track {
			id: sample.id,
			origin_x: sample.x,
			origin_y: sample.y,
			x: sample.x,
			y: sample.y,
			down_ms: sample.time_ms,
			last_ms: sample.time_ms,
			kind: sample.kind,
			members,
			samples: [(0, 0.0, 0.0); 8],
			sample_at: 0,
			sample_len: 0,
			claimed: false,
		};
		push_sample(&mut track);
		self.note_spans(&track);
		self.tracks.push(track);
		self.resolve(sample.id, PointerPhase::Down, sample.time_ms);
		self.wake_others(sample.id, PointerPhase::Down, sample.time_ms);
	}

	fn relocate(&mut self, sample: PointerSample, up: bool) {
		let Some(index) = self.tracks.iter().position(|track| track.id == sample.id) else {
			return;
		};

		{
			let track = &mut self.tracks[index];
			track.x = sample.x;
			track.y = sample.y;
			track.last_ms = sample.time_ms;
			push_sample(track);
		}

		self.move_owned_span(sample.id);
		let phase = if up {
			PointerPhase::Up
		} else {
			PointerPhase::Move
		};
		self.resolve(sample.id, phase, sample.time_ms);

		if !up {
			self.wake_others(sample.id, phase, sample.time_ms);
		}

		if up {
			self.finish(sample.id, sample.time_ms);
		}
	}

	fn resolve(&mut self, id: u64, phase: PointerPhase, now: u64) {
		let Some(index) = self.tracks.iter().position(|track| track.id == id) else {
			return;
		};

		let mut accepting = Vec::new();

		for member_index in 0..self.tracks[index].members.len() {
			if self.tracks[index].members[member_index].phase != Phase::Possible {
				continue;
			}

			let intent = member_intent(&self.tracks[index], member_index, &self.spans, phase, now);

			if matches!(intent, Intent::Reject) {
				self.tracks[index].members[member_index].phase = Phase::Failed;
			} else if matches!(intent, Intent::Accept) {
				accepting.push(member_index);
			}
		}

		accepting.sort_by_key(|member_index| {
			u8::from(!self.tracks[index].members[*member_index].preempt())
		});

		for member_index in accepting {
			if self.tracks[index].members[member_index].phase != Phase::Possible {
				continue;
			}

			if !gate_open(&self.tracks[index].members, member_index) {
				continue;
			}

			self.accept(index, member_index);
		}

		if phase == PointerPhase::Move {
			self.emit_updates(index);
		}

		self.tracks[index].claimed = self.tracks[index]
			.members
			.iter()
			.any(|member| member.phase == Phase::Active && member.claims());
	}

	fn accept(&mut self, track_index: usize, member_index: usize) {
		let event = self.event_for(track_index, member_index, GestureState::Began);
		let finishes = self.tracks[track_index].members[member_index].finishes_on_accept();
		self.tracks[track_index].members[member_index].phase =
			if finishes { Phase::Done } else { Phase::Active };
		self.emit_call(track_index, member_index, true, event);

		if finishes {
			let end = GestureEvent {
				state: GestureState::End,
				..event
			};
			self.emit_call(track_index, member_index, false, end);
		}

		self.reject_losers(track_index, member_index);
	}

	fn reject_losers(&mut self, track_index: usize, winner: usize) {
		let winner_member = self.tracks[track_index].members[winner].clone_keys();
		let keep_button = self.tracks[track_index].members[winner].is_tap();
		let mut reject = Vec::new();

		for (index, member) in self.tracks[track_index].members.iter().enumerate() {
			if index == winner || matches!(member.phase, Phase::Failed | Phase::Done) {
				continue;
			}

			if keep_button && member.is_button() {
				continue;
			}

			let same_gate = member.gate != 0
				&& member.gate == winner_member.gate
				&& member.priority != winner_member.priority;

			if same_gate || conflicts(&winner_member, member) {
				reject.push(index);
			}
		}

		for index in reject {
			self.fail_member(track_index, index);
		}
	}

	fn fail_member(&mut self, track_index: usize, member_index: usize) {
		let phase = self.tracks[track_index].members[member_index].phase;

		if matches!(phase, Phase::Failed | Phase::Done) {
			return;
		}

		self.tracks[track_index].members[member_index].phase = Phase::Failed;

		if phase == Phase::Active {
			let event = self.event_for(track_index, member_index, GestureState::Cancelled);
			self.emit_named(track_index, member_index, event, |calls| {
				calls.cancel.clone()
			});
		}
	}

	fn emit_updates(&mut self, track_index: usize) {
		for member_index in 0..self.tracks[track_index].members.len() {
			let member = &self.tracks[track_index].members[member_index];

			if member.phase != Phase::Active || !member.reports_updates() {
				continue;
			}

			if !self.primary_update(track_index, member_index) {
				continue;
			}

			let event = self.event_for(track_index, member_index, GestureState::Active);
			self.emit_named(track_index, member_index, event, |calls| {
				calls.update.clone()
			});
		}
	}

	fn finish(&mut self, id: u64, _now: u64) {
		let Some(index) = self.tracks.iter().position(|track| track.id == id) else {
			return;
		};

		self.drop_span_point(id);
		let mut ending = Vec::new();

		for (member_index, member) in self.tracks[index].members.iter().enumerate() {
			if member.phase == Phase::Active {
				ending.push(member_index);
			}
		}

		for member_index in ending {
			self.tracks[index].members[member_index].phase = Phase::Done;
			let event = self.event_for(index, member_index, GestureState::End);
			self.emit_named(index, member_index, event, |calls| calls.end.clone());
		}

		self.tracks.remove(index);
		self.spans.retain(|span| span.count > 0);
	}

	fn cancel_pointer(&mut self, id: u64, _now: u64) {
		let Some(index) = self.tracks.iter().position(|track| track.id == id) else {
			return;
		};

		let mut cancelling = Vec::new();

		for (member_index, member) in self.tracks[index].members.iter().enumerate() {
			if member.phase == Phase::Active {
				cancelling.push(member_index);
			}
		}

		for member_index in cancelling {
			self.tracks[index].members[member_index].phase = Phase::Failed;
			let event = self.event_for(index, member_index, GestureState::Cancelled);
			self.emit_named(index, member_index, event, |calls| calls.cancel.clone());
		}

		self.drop_span_point(id);
		self.tracks.remove(index);
		self.spans.retain(|span| span.count > 0);
	}

	fn emit_call(
		&mut self,
		track_index: usize,
		member_index: usize,
		begin: bool,
		event: GestureEvent,
	) {
		let pick = if begin {
			|calls: &Calls| calls.begin.clone()
		} else {
			|calls: &Calls| calls.end.clone()
		};
		self.emit_named(track_index, member_index, event, pick);
	}

	fn emit_named(
		&mut self,
		track_index: usize,
		member_index: usize,
		event: GestureEvent,
		pick: impl Fn(&Calls) -> Option<Callback>,
	) {
		let Some(callback) = pick(&self.tracks[track_index].members[member_index].calls) else {
			return;
		};

		if !self.primary_update(track_index, member_index) {
			return;
		}

		self.push_effect(callback, event);
	}

	fn primary_update(&self, track_index: usize, member_index: usize) -> bool {
		let member = &self.tracks[track_index].members[member_index];

		if !member.is_span() {
			return true;
		}

		let pointer = self.tracks[track_index].id;
		self.spans
			.iter()
			.find(|span| span.node == member.node)
			.is_none_or(|span| span.primary == pointer)
	}

	fn event_for(
		&self,
		track_index: usize,
		member_index: usize,
		state: GestureState,
	) -> GestureEvent {
		let track = &self.tracks[track_index];
		let member = &track.members[member_index];
		let (velocity_x, velocity_y) = velocity(track);
		let span = self.spans.iter().find(|span| span.node == member.node);
		let (x, y, translation_x, translation_y, scale, rotation, pointers) =
			gesture_geometry(track, span);
		GestureEvent {
			state,
			x,
			y,
			translation_x,
			translation_y,
			velocity_x,
			velocity_y,
			scale,
			rotation,
			pointers,
			pointer: track.id,
			kind: track.kind,
		}
	}

	fn push_effect(&mut self, callback: Callback, event: GestureEvent) {
		self.effects.push(Effect { callback, event });
	}

	fn decide(&self, now: u64) -> PointerDecision {
		let route = self
			.tracks
			.iter()
			.fold(PointerRoute::Platform, |route, track| {
				stronger(route, track_route(track))
			});
		PointerDecision {
			route,
			deadline_ms: self.deadline(now),
		}
	}

	fn deadline(&self, now: u64) -> Option<u64> {
		let mut soonest = None;

		for track in &self.tracks {
			for member in &track.members {
				if member.phase != Phase::Possible {
					continue;
				}

				let at = member.deadline(track.down_ms);

				if let Some(at) = at.filter(|at| *at > now) {
					soonest = Some(soonest.map_or(at, |current: u64| current.min(at)));
				}
			}
		}

		soonest
	}

	fn note_spans(&mut self, track: &Track) {
		for member in &track.members {
			if !member.is_span() {
				continue;
			}

			if let Some(span) = self.spans.iter_mut().find(|span| span.node == member.node) {
				span.upsert(track.id, track.x, track.y);
			} else {
				let mut span = Span::new(member.node, track.id);
				span.upsert(track.id, track.x, track.y);
				self.spans.push(span);
			}
		}
	}

	fn move_owned_span(&mut self, id: u64) {
		let Some(track) = self.tracks.iter().find(|track| track.id == id) else {
			return;
		};
		let nodes: Vec<NodeId> = track
			.members
			.iter()
			.filter(|member| member.is_span())
			.map(|member| member.node)
			.collect();
		let (x, y) = (track.x, track.y);

		for span in &mut self.spans {
			if nodes.contains(&span.node) {
				span.upsert(id, x, y);
			}
		}
	}

	fn wake_others(&mut self, id: u64, phase: PointerPhase, now: u64) {
		let nodes: Vec<NodeId> = self
			.tracks
			.iter()
			.find(|track| track.id == id)
			.map(|track| {
				track
					.members
					.iter()
					.filter(|member| member.is_span())
					.map(|member| member.node)
					.collect()
			})
			.unwrap_or_default();

		if nodes.is_empty() {
			return;
		}

		let others: Vec<u64> = self
			.tracks
			.iter()
			.filter(|track| track.id != id)
			.filter(|track| {
				track
					.members
					.iter()
					.any(|member| member.is_span() && nodes.contains(&member.node))
			})
			.map(|track| track.id)
			.collect();

		for other in others {
			self.resolve(other, phase, now);
		}
	}

	fn drop_span_point(&mut self, id: u64) {
		for span in &mut self.spans {
			span.remove(id);

			if span.count > 0 && span.primary == id {
				span.primary = span.points[0].0;
			}
		}
	}
}

struct MemberKeys {
	team: u16,
	gate: u16,
	priority: u16,
	contest: u16,
}

impl Member {
	fn clone_keys(&self) -> MemberKeys {
		MemberKeys {
			team: self.team,
			gate: self.gate,
			priority: self.priority,
			contest: self.contest,
		}
	}

	fn preempt(&self) -> bool {
		matches!(self.leaf, Leaf::Pan { preempt: true, .. })
	}

	fn claims(&self) -> bool {
		matches!(
			self.leaf,
			Leaf::Pan { .. } | Leaf::LongPress { .. } | Leaf::Pinch | Leaf::Rotation
		)
	}

	fn finishes_on_accept(&self) -> bool {
		matches!(
			self.leaf,
			Leaf::Tap { .. } | Leaf::Fling { .. } | Leaf::Native { .. }
		)
	}

	fn reports_updates(&self) -> bool {
		matches!(
			self.leaf,
			Leaf::Pan { .. } | Leaf::LongPress { .. } | Leaf::Pinch | Leaf::Rotation
		)
	}

	fn is_tap(&self) -> bool {
		matches!(self.leaf, Leaf::Tap { .. })
	}

	fn is_button(&self) -> bool {
		matches!(self.leaf, Leaf::Native { drag: false })
	}

	fn is_span(&self) -> bool {
		matches!(self.leaf, Leaf::Pinch | Leaf::Rotation)
	}

	fn deadline(&self, down_ms: u64) -> Option<u64> {
		match &self.leaf {
			Leaf::LongPress { duration_ms, .. } => Some(down_ms.saturating_add(*duration_ms)),
			Leaf::Tap {
				max_ms: Some(max_ms),
				..
			} => Some(down_ms.saturating_add(*max_ms)),
			_ => None,
		}
	}
}

fn conflicts(winner: &MemberKeys, other: &Member) -> bool {
	if winner.contest != 0 && winner.contest == other.contest {
		return true;
	}

	if winner.team != 0 && winner.team == other.team {
		return false;
	}

	true
}

fn gate_open(members: &[Member], index: usize) -> bool {
	let member = &members[index];

	if member.gate == 0 {
		return true;
	}

	members
		.iter()
		.filter(|other| other.gate == member.gate && other.priority < member.priority)
		.all(|other| matches!(other.phase, Phase::Failed | Phase::Done))
}

fn member_intent(
	track: &Track,
	index: usize,
	spans: &[Span],
	phase: PointerPhase,
	now: u64,
) -> Intent {
	let member = &track.members[index];
	let (dx, dy) = (track.x - track.origin_x, track.y - track.origin_y);
	let distance = dx.hypot(dy);

	match &member.leaf {
		Leaf::Tap { slop, max_ms } => {
			tap_intent(*slop, *max_ms, distance, track.down_ms, now, phase)
		}
		Leaf::LongPress { slop, duration_ms } => {
			long_intent(*slop, *duration_ms, distance, track.down_ms, now, phase)
		}
		Leaf::Pan { slop, axis, .. } => pan_intent(*slop, *axis, dx, dy, distance, phase),
		Leaf::Pinch | Leaf::Rotation => span_intent(spans, member.node, phase),
		Leaf::Fling { min_speed } => fling_intent(*min_speed, track, phase),
		Leaf::Native { drag } => native_intent(*drag, distance, phase),
	}
}

fn tap_intent(
	slop: f32,
	max_ms: Option<u64>,
	distance: f32,
	down_ms: u64,
	now: u64,
	phase: PointerPhase,
) -> Intent {
	if distance > slop || max_ms.is_some_and(|max_ms| now.saturating_sub(down_ms) > max_ms) {
		return Intent::Reject;
	}

	if phase == PointerPhase::Up {
		Intent::Accept
	} else {
		Intent::Wait
	}
}

fn long_intent(
	slop: f32,
	duration_ms: u64,
	distance: f32,
	down_ms: u64,
	now: u64,
	phase: PointerPhase,
) -> Intent {
	if distance > slop || phase == PointerPhase::Up {
		return Intent::Reject;
	}

	if now.saturating_sub(down_ms) >= duration_ms {
		Intent::Accept
	} else {
		Intent::Wait
	}
}

fn pan_intent(
	slop: f32,
	axis: Axis,
	dx: f32,
	dy: f32,
	distance: f32,
	phase: PointerPhase,
) -> Intent {
	if axis_lost(axis, dx, dy, slop) || (phase == PointerPhase::Up && distance < slop) {
		return Intent::Reject;
	}

	if distance >= slop {
		Intent::Accept
	} else {
		Intent::Wait
	}
}

fn axis_lost(axis: Axis, dx: f32, dy: f32, slop: f32) -> bool {
	match axis {
		Axis::Free => false,
		Axis::Horizontal => dy.abs() > slop && dy.abs() > dx.abs(),
		Axis::Vertical => dx.abs() > slop && dx.abs() > dy.abs(),
	}
}

fn span_intent(spans: &[Span], node: NodeId, phase: PointerPhase) -> Intent {
	let count = spans
		.iter()
		.find(|span| span.node == node)
		.map_or(0, |span| span.count);

	if count >= 2 {
		Intent::Accept
	} else if phase == PointerPhase::Up {
		Intent::Reject
	} else {
		Intent::Wait
	}
}

fn fling_intent(min_speed: f32, track: &Track, phase: PointerPhase) -> Intent {
	if phase != PointerPhase::Up {
		return Intent::Wait;
	}

	let (vx, vy) = velocity(track);

	if vx.hypot(vy) >= min_speed {
		Intent::Accept
	} else {
		Intent::Reject
	}
}

fn native_intent(drag: bool, distance: f32, phase: PointerPhase) -> Intent {
	if drag && distance >= TOUCH_SLOP {
		return Intent::Accept;
	}

	if !drag && distance > TOUCH_SLOP {
		return Intent::Reject;
	}

	if phase == PointerPhase::Up {
		Intent::Accept
	} else {
		Intent::Wait
	}
}

fn track_route(track: &Track) -> PointerRoute {
	if track.claimed {
		PointerRoute::Claim
	} else if track
		.members
		.iter()
		.any(|member| matches!(member.phase, Phase::Possible | Phase::Active))
	{
		PointerRoute::Track
	} else {
		PointerRoute::Platform
	}
}

fn stronger(left: PointerRoute, right: PointerRoute) -> PointerRoute {
	match (left, right) {
		(PointerRoute::Claim, _) | (_, PointerRoute::Claim) => PointerRoute::Claim,
		(PointerRoute::Track, _) | (_, PointerRoute::Track) => PointerRoute::Track,
		_ => PointerRoute::Platform,
	}
}

fn gesture_geometry(track: &Track, span: Option<&Span>) -> (f32, f32, f32, f32, f32, f32, u8) {
	if let Some(span) = span.filter(|span| span.count >= 2) {
		let (fx, fy) = span.focal();
		return (
			fx,
			fy,
			fx - span.focal_x,
			fy - span.focal_y,
			span.scale,
			span.rotation,
			span.count,
		);
	}

	(
		track.x,
		track.y,
		track.x - track.origin_x,
		track.y - track.origin_y,
		1.0,
		0.0,
		1,
	)
}

fn velocity(track: &Track) -> (f32, f32) {
	if track.sample_len < 2 {
		return (0.0, 0.0);
	}

	let len = track.sample_len as usize;
	let newest = sample_at(track, len - 1);
	let (t1, x1, y1) = newest;
	let mut chosen = None;

	for step in 0..len {
		let (t, x, y) = sample_at(track, step);

		if t1.saturating_sub(t) <= VELOCITY_WINDOW_MS {
			chosen.get_or_insert((t, x, y));
		}
	}

	let Some((t0, x0, y0)) = chosen else {
		return (0.0, 0.0);
	};
	let dt = t1.saturating_sub(t0) as f32 / 1000.0;

	if dt <= 0.0 {
		return (0.0, 0.0);
	}

	((x1 - x0) / dt, (y1 - y0) / dt)
}

fn sample_at(track: &Track, step: usize) -> (u64, f32, f32) {
	let len = usize::from(track.sample_len);
	let oldest = if len == 8 {
		usize::from(track.sample_at)
	} else {
		0
	};
	track.samples[(oldest + step) % 8]
}

fn push_sample(track: &mut Track) {
	let index = usize::from(track.sample_at);
	track.samples[index] = (track.last_ms, track.x, track.y);
	track.sample_at = (track.sample_at + 1) % 8;
	track.sample_len = track.sample_len.saturating_add(1).min(8);
}

fn members_from(collected: Vec<Collected>) -> Vec<Member> {
	let mut out = Vec::new();
	let mut ids = Ids { next: 1 };

	for item in collected {
		if let Some(gesture) = &item.gesture {
			flatten(gesture, item.node, &mut out, &mut ids, Flatten::default());
		}

		if let Some(drag) = item.native
			&& !out.iter().any(|member| {
				member.node == item.node && matches!(member.leaf, Leaf::Native { .. })
			}) {
			out.push(member(
				item.node,
				LeafSpec {
					leaf: Leaf::Native { drag },
					calls: Calls::default(),
				},
				Flatten::default(),
			));
		}
	}

	out
}

struct Ids {
	next: u16,
}

impl Ids {
	fn take(&mut self) -> u16 {
		let id = self.next;
		self.next = self.next.saturating_add(1);
		id
	}
}

#[derive(Clone, Copy, Default)]
struct Flatten {
	team: u16,
	gate: u16,
	priority: u16,
	contest: u16,
}

fn flatten(
	gesture: &Gesture,
	node: NodeId,
	out: &mut Vec<Member>,
	ids: &mut Ids,
	inherited: Flatten,
) {
	match &gesture.node {
		GestureNode::Group { mode, children } => {
			flatten_group(*mode, children, node, out, ids, inherited);
		}
		GestureNode::Tap {
			slop,
			max_ms,
			calls,
		} => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::Tap {
						slop: *slop,
						max_ms: *max_ms,
					},
					calls: calls.clone(),
				},
				inherited,
			))
		}
		GestureNode::LongPress {
			slop,
			duration_ms,
			calls,
		} => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::LongPress {
						slop: *slop,
						duration_ms: *duration_ms,
					},
					calls: calls.clone(),
				},
				inherited,
			))
		}
		GestureNode::Pan {
			slop,
			axis,
			preempt,
			calls,
		} => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::Pan {
						slop: *slop,
						axis: *axis,
						preempt: *preempt,
					},
					calls: calls.clone(),
				},
				inherited,
			))
		}
		GestureNode::Pinch { calls } => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::Pinch,
					calls: calls.clone(),
				},
				inherited,
			))
		}
		GestureNode::Rotation { calls } => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::Rotation,
					calls: calls.clone(),
				},
				inherited,
			))
		}
		GestureNode::Fling { min_speed, calls } => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::Fling {
						min_speed: *min_speed,
					},
					calls: calls.clone(),
				},
				inherited,
			))
		}
		GestureNode::Native => {
			out.push(member(
				node,
				LeafSpec {
					leaf: Leaf::Native { drag: false },
					calls: Calls::default(),
				},
				inherited,
			))
		}
	}
}

fn flatten_group(
	mode: GroupMode,
	children: &[Gesture],
	node: NodeId,
	out: &mut Vec<Member>,
	ids: &mut Ids,
	inherited: Flatten,
) {
	match mode {
		GroupMode::Simultaneous => {
			let team = ids.take();
			let mut scope = inherited;
			scope.team = team;

			for child in children {
				flatten(child, node, out, ids, scope);
			}
		}
		GroupMode::Exclusive => {
			let gate = ids.take();

			for (priority, child) in children.iter().enumerate() {
				let mut scope = inherited;
				scope.gate = gate;
				scope.priority = u16::try_from(priority).unwrap_or(u16::MAX);
				flatten(child, node, out, ids, scope);
			}
		}
		GroupMode::Race => {
			let contest = ids.take();
			let mut scope = inherited;
			scope.contest = contest;

			for child in children {
				flatten(child, node, out, ids, scope);
			}
		}
	}
}

fn member(node: NodeId, spec: LeafSpec, scope: Flatten) -> Member {
	Member {
		node,
		leaf: spec.leaf,
		calls: spec.calls,
		team: scope.team,
		gate: scope.gate,
		priority: scope.priority,
		contest: scope.contest,
		phase: Phase::Possible,
	}
}

impl Span {
	fn new(node: NodeId, primary: u64) -> Self {
		Self {
			node,
			primary,
			points: [(0, 0.0, 0.0); 2],
			count: 0,
			base_len: 1.0,
			angle: 0.0,
			rotation: 0.0,
			scale: 1.0,
			focal_x: 0.0,
			focal_y: 0.0,
		}
	}

	fn upsert(&mut self, id: u64, x: f32, y: f32) {
		if let Some(slot) = self
			.points
			.iter_mut()
			.take(self.count as usize)
			.find(|point| point.0 == id)
		{
			slot.1 = x;
			slot.2 = y;
			self.refresh();
			return;
		}

		if self.count >= 2 {
			return;
		}

		self.points[self.count as usize] = (id, x, y);
		self.count += 1;

		if self.count == 2 {
			self.capture();
		}
	}

	fn remove(&mut self, id: u64) {
		let Some(index) = self
			.points
			.iter()
			.take(self.count as usize)
			.position(|point| point.0 == id)
		else {
			return;
		};

		if index == 0 && self.count == 2 {
			self.points[0] = self.points[1];
		}

		self.count -= 1;
	}

	fn capture(&mut self) {
		let (dx, dy) = self.delta();
		self.base_len = dx.hypot(dy).max(0.000_1);
		self.angle = dy.atan2(dx);
		self.rotation = 0.0;
		self.scale = 1.0;
		let (fx, fy) = self.focal();
		self.focal_x = fx;
		self.focal_y = fy;
	}

	fn refresh(&mut self) {
		if self.count < 2 {
			return;
		}

		let (dx, dy) = self.delta();
		let len = dx.hypot(dy);
		self.scale = len / self.base_len;
		let angle = dy.atan2(dx);
		let mut turn = angle - self.angle;

		if turn > std::f32::consts::PI {
			turn -= std::f32::consts::TAU;
		} else if turn < -std::f32::consts::PI {
			turn += std::f32::consts::TAU;
		}

		self.rotation += turn;
		self.angle = angle;
	}

	fn delta(&self) -> (f32, f32) {
		(
			self.points[1].1 - self.points[0].1,
			self.points[1].2 - self.points[0].2,
		)
	}

	fn focal(&self) -> (f32, f32) {
		if self.count == 0 {
			return (0.0, 0.0);
		}

		let count = f32::from(self.count);
		let (x, y) = self
			.points
			.iter()
			.take(self.count as usize)
			.fold((0.0, 0.0), |total, point| {
				(total.0 + point.1, total.1 + point.2)
			});
		(x / count, y / count)
	}
}
