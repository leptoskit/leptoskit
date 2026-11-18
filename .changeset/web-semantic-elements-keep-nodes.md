---
"leptoskit_native": patch
---

# Semantic web elements keep their nodes

On the web, a view with a semantic role (`main`, `nav`, a list, a heading, `header`, `footer`, `aside`, `article`, `section`, `search`, a paragraph) was replaced by a new element every time its parent rendered again. A text field inside one lost focus and its cursor on each keystroke that changed state. These views now render through typed elements, so the element is updated in place and its children keep their nodes, focus, and scroll position.
