//! Stress screens, fixtures, and the Windows desk.

use leptoskit::prelude::*;
use leptoskit::virtual_list::Axis;
use leptoskit::virtual_list::VirtualList;
use leptoskit::virtual_list::VirtualOptions;
use leptoskit::virtual_list::Virtualizer;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

/// A window into a hundred thousand messages.
#[component]
pub fn MessagesStress() -> Element {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(100_000, 44.0).with_overscan(2));
	virtualizer.set_viewport(640.0);
	let list = VirtualList(
		&virtualizer,
		Axis::Vertical,
		|_| {},
		|item| {
			let label = format!("Message {}", item.index + 1);
			view! { <Text class="h-8 px-1 text-sm text-slate-900">{label}</Text> }
		},
	)
	.expect("a fixed row is a valid estimate");
	view! {
		<Frame title="Messages stress">
			<Note text="100,000 messages. The window is the only thing built." />
			<View class="h-[240px]">{list}</View>
		</Frame>
	}
}

/// One hundred and twenty-eight requests, finished together.
#[component]
pub fn CompletionStorm() -> Element {
	let done = remember(|| false);
	let finished = done.get();
	let status = if finished { "Done" } else { "Waiting" };
	view! {
		<Frame title="Completion storm">
			<Note text="128 requests. Eight of them stand in for the rest." />
			<Choice label="Run" on_press=move || done.set(true) />
			<Text class="text-sm font-semibold text-slate-900">{status}</Text>
			<Text class="text-xs text-slate-500">"128 slots"</Text>
		</Frame>
	}
}

/// A stand-in for a 4 MiB document: the shape, not the bytes.
#[component]
pub fn MarkdownStress() -> Element {
	view! {
		<Frame title="Markdown stress">
			<Note text="A 4 MiB document. This screen shows the first slices." />
			<Card>
				<Text class="text-sm text-slate-900">"Paragraph · the opening"</Text>
				<Text class="text-sm text-slate-900">"Code · a fenced block"</Text>
				<Text class="text-sm text-slate-900">"Table · three columns"</Text>
			</Card>
		</Frame>
	}
}

/// The native-module seam, as a panel the host would fill.
#[component]
pub fn NativeFixture() -> Element {
	let ready = remember(|| false);
	let on = ready.get();
	view! {
		<Frame title="Native fixture">
			<Note text="A hyphenated tag. The host draws the module inside it." />
			<Card>
				<Text class="text-sm font-semibold text-slate-900">"exact-map"</Text>
				<Text class="text-sm text-slate-600">
					{if on { "Module ready" } else { "Waiting" }}
				</Text>
				<Choice label="Attach" on_press=move || ready.set(true) />
			</Card>
		</Frame>
	}
}

const AUTH_STEPS: [&str; 3] = ["PAR", "DPoP", "PKCE"];

/// The OAuth steps, advanced one at a time.
#[component]
pub fn AuthFixture() -> Element {
	let step = remember(|| 0_usize);
	let index = step.get();
	view! {
		<Frame title="Auth fixture">
			<Note text="PAR, DPoP, and PKCE, against a local server in the original." />
			<Card>
				<Text class="text-base font-semibold text-slate-900">{AUTH_STEPS[index]}</Text>
				<Choice
					label="Continue"
					on_press=move || step.set((index + 1) % AUTH_STEPS.len())
				/>
			</Card>
		</Frame>
	}
}

/// Native controls over a Contract-shaped desk.
#[component]
pub fn WindowsDesk() -> Element {
	let armed = remember(|| false);
	let on = armed.get();
	view! {
		<Frame title="Windows desk">
			<Note text="A button and a switch, the controls a Win32 desk draws." />
			<Card>
				<Text class="text-sm text-slate-900">{if on { "Armed" } else { "Idle" }}</Text>
				<Choice label="Arm" on_press=move || armed.set(true) />
			</Card>
		</Frame>
	}
}
