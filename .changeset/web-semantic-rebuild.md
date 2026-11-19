---
"leptoskit_native": patch
"leptoskit": patch
---

# Keep focus across web rebuilds and inside modals

On the web, a screen that rebuilt replaced every landmark (`main`, `nav`, `header`, `footer`, `aside`, `article`, `section`, `search`), every heading, list, paragraph and label with a new element, instead of updating the one already there. Anything inside went with it: a text field lost its focus and its selection, and a person typing was dropped back to the top of the page.

Those elements now update in place, as `div` and `span` always did, so focus stays where it was after any rebuild.

A rebuild also no longer takes a node's `id`, `role`, `aria-*` and `tabindex` attributes off and puts them back: it sets the ones whose value changed and removes the ones that are gone. Before, a focused element that is only focusable through `tabindex`, such as a dialog's heading, lost focus to the page on every rebuild.

A `Modal` on the web now wraps Tab: past its last control focus comes back to its first, and Shift+Tab past its first goes to its last, instead of leaving the page for the browser's toolbar.
