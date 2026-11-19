//! The pages this crate builds, each framed by the [`Shell`].

mod glossary;
mod learn;

use leptoskit::prelude::Element;

use crate::model::Page;
use crate::shell::Shell;

/// The function that builds the page's tree, with the page's own state made
/// once beside it.
pub(crate) fn screen_of(shell: &Shell) -> Box<dyn Fn() -> Element + Send + Sync> {
	let shell = shell.clone();

	Box::new(move || shell.frame(content(&shell.model.page)))
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
	}
}
