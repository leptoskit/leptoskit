//! The phone frame every Exact example shares.

use leptoskit::prelude::*;

/// A phone-sized column with the example's name.
#[component]
pub fn Frame(title: &'static str, children: Children) -> Element {
	view! {
		<View class="w-[390px] h-[844px] bg-slate-50">
			<View class="gap-1 px-4 pt-4 pb-3 bg-white">
				<Text class="text-xs font-semibold text-indigo-600">"leptoskit · exact"</Text>
				<Heading aria_level=1 class="text-2xl font-bold text-slate-900">{title}</Heading>
			</View>
			<ScrollView class="flex-1">
				<View class="gap-3 p-4">{children}</View>
			</ScrollView>
		</View>
	}
}

/// A tappable choice whose label is its own text.
#[component]
pub fn Choice(label: &'static str, on_press: impl Fn() + Send + Sync + 'static) -> Element {
	view! {
		<Button
			class="px-3 py-2 rounded-lg bg-indigo-500 text-sm font-semibold text-white"
			on:press=on_press
		>
			{label}
		</Button>
	}
}

/// Body copy under a frame.
#[component]
pub fn Note(text: &'static str) -> Element {
	view! { <Text class="text-sm text-slate-600">{text}</Text> }
}

/// A white card of stacked children.
#[component]
pub fn Card(children: Children) -> Element {
	view! { <View class="gap-2 rounded-xl bg-white p-4">{children}</View> }
}
