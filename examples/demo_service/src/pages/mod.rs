//! The one page the service still draws itself: "opening today's demo",
//! which it serves while a generation builds. The front door, sign-in, and
//! Notes are `demo_pages`, rendered here and hydrated in the browser.
//!
//! The page is a real leptoskit component tree, written as `view!` markup
//! and rendered by the same web renderer the admin uses, in the admin's
//! paper, ink, ember, and mint ([`leptoskit_admin::theme_css`]). It carries
//! nothing about the visitor, needs no script, and is `no-store`, which the
//! route sets.

mod opening;

pub(crate) use opening::RETRY_SECONDS;
pub(crate) use opening::opening;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_opening_page_says_what_is_known() {
		let html = opening();
		assert!(html.contains("opening today"));
		assert!(html.contains(&format!("content=\"{RETRY_SECONDS}\"")));
		assert!(!html.contains("<script"), "the page needs no script");
	}
}
