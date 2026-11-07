use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Update;
use leptoskit::prelude::*;

pub fn app() -> impl Fn() -> Element {
	let count = RwSignal::new(0);
	move || {
		view! {
			<View class="p-8 gap-4 flex-col">
				<Text>"A small idea starts here"</Text>
				<Button on:press=move || count.update(|n| *n += 1)>"One more idea"</Button>
				<Text>{format!("{} ideas shipped", count.get())}</Text>
			</View>
		}
	}
}
