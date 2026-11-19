//! The pages this crate builds, each framed by the [`Shell`].

mod glossary;
mod infra;
mod learn;
mod library;
mod people;
mod project;

use leptoskit::prelude::Element;

use crate::model::Page;
use crate::shell::Shell;

/// The function that builds the page's tree, with the page's own state made
/// once beside it.
pub(crate) fn screen_of(shell: &Shell) -> Box<dyn Fn() -> Element + Send + Sync> {
	let shell = shell.clone();

	match &shell.model.page {
		Page::Operation(operation) if operation.running => {
			let follow = infra::Follow::open(&operation.id);
			Box::new(move || {
				let Page::Operation(operation) = &shell.model.page else {
					return shell.frame(content(&shell.model.page));
				};
				shell.frame(infra::operation(operation, Some(follow)))
			})
		}
		Page::Project(project) => {
			let actions = project::Actions::new(&project.id, shell.confirm, shell.notice);

			if let crate::model::project::Body::Changes(changes) = &project.body
				&& changes.workings_by_default
			{
				// The plan's Workings open by default the first three times.
				crate::shell::preference(r#"{"kind":"use_note","note":"plan-workings"}"#);
			}

			Box::new(move || {
				let Page::Project(project) = &shell.model.page else {
					return shell.frame(content(&shell.model.page));
				};
				shell.frame(project::page(project, Some(actions)))
			})
		}
		Page::People(people) => {
			let search = people::Search::new(people);
			Box::new(move || {
				let Page::People(people) = &shell.model.page else {
					return shell.frame(content(&shell.model.page));
				};
				shell.frame(people::list(people, Some(search)))
			})
		}
		Page::Person(person) => {
			let actions = people::Actions::new(person, shell.confirm, shell.notice);
			Box::new(move || {
				let Page::Person(person) = &shell.model.page else {
					return shell.frame(content(&shell.model.page));
				};
				shell.frame(people::person(person, Some(actions)))
			})
		}
		_ => Box::new(move || shell.frame(content(&shell.model.page))),
	}
}

/// The content of `page`, without the frame, for a page that has no state
/// of its own. The admin uses it for the parts of a page it still renders
/// itself.
#[must_use]
pub fn content(page: &Page) -> Element {
	match page {
		Page::Glossary => glossary::content(),
		Page::Learn(learn) => learn::hub(learn),
		Page::Lesson(lesson) => learn::lesson(lesson),
		Page::Projects(cards) => infra::projects(cards),
		Page::Operation(operation) => infra::operation(operation, None),
		Page::Project(project) => project::page(project, None),
		Page::Library(library) => library::page(library),
		Page::People(people) => people::list(people, None),
		Page::Person(person) => people::person(person, None),
	}
}
