//! Hello, the todo from Exact's guide, and Shelf.

use leptoskit::prelude::*;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

/// The screen `exact new` starts from: a greeting and a counter.
#[component]
pub fn Hello() -> Element {
	let taps = remember(|| 0_u32);
	let count = taps.get();
	let tap = move || taps.update(|value| *value += 1);
	view! {
		<Frame title="Hello">
			<Note text="One tree of components, the same screen Exact's starter opens." />
			<Card>
				<Text class="text-base text-slate-900">{format!("Taps: {count}")}</Text>
				<Choice label="Say hello" on_press=tap />
			</Card>
		</Frame>
	}
}

/// The todo from Exact's guide: add, finish, and delete stay in one list.
#[component]
pub fn Todo() -> Element {
	let items = remember(|| vec![Task::new(1, "Buy milk"), Task::new(2, "Walk the dog")]);
	let next = remember(|| 3_u32);
	let tasks = items.get();
	let left = tasks.iter().filter(|task| !task.done).count();
	let add = move || {
		let id = next.get();
		next.set(id + 1);
		items.update(|tasks| tasks.push(Task::new(id, "Write a postcard")));
	};
	view! {
		<Frame title="Todo">
			<Text class="text-base font-semibold text-slate-900">{format!("{left} left")}</Text>
			<Choice label="Add" on_press=add />
			{tasks
				.into_iter()
				.map(|task| {
					let toggle = move || {
						items.update(|tasks| {
							if let Some(found) = tasks.iter_mut().find(|item| item.id == task.id) {
								found.done = !found.done;
							}
						});
					};
					let remove = move || {
						items.update(|tasks| tasks.retain(|item| item.id != task.id));
					};
					view! { <TaskRow task=task on_toggle=toggle on_remove=remove /> }
				})
				.collect::<Fragment>()}
		</Frame>
	}
}

/// A recipe shelf that leaves colour and type to the platform frame.
#[component]
pub fn Shelf() -> Element {
	let picked = remember(|| 0_usize);
	let index = picked.get();
	let recipes = [
		("Tomato noodles", "Garlic, tomatoes, noodles"),
		("Citrus salad", "Orange, fennel, olive oil"),
		("Night broth", "Ginger, scallion, pepper"),
	];
	view! {
		<Frame title="Shelf">
			<Note text="Recipes, with no colour and no font size of their own." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Tomato noodles" on_press=move || picked.set(0) />
				<Choice label="Citrus salad" on_press=move || picked.set(1) />
				<Choice label="Night broth" on_press=move || picked.set(2) />
			</View>
			<Card>
				<Text class="text-base font-semibold text-slate-900">{recipes[index].0}</Text>
				<Text class="text-sm text-slate-600">{recipes[index].1}</Text>
			</Card>
		</Frame>
	}
}

#[derive(Clone, Copy)]
struct Task {
	id: u32,
	title: &'static str,
	done: bool,
}

impl Task {
	fn new(id: u32, title: &'static str) -> Self {
		Self {
			id,
			title,
			done: false,
		}
	}
}

#[component]
fn TaskRow(
	task: Task,
	on_toggle: impl Fn() + Send + Sync + 'static,
	on_remove: impl Fn() + Send + Sync + 'static,
) -> Element {
	let mark = if task.done { "Done" } else { "Open" };
	view! {
		<Card>
			<Text class="text-sm text-slate-900">{task.title}</Text>
			<View class="flex-row gap-2">
				<Choice label=mark on_press=on_toggle />
				<Choice label="Delete" on_press=on_remove />
			</View>
		</Card>
	}
}
