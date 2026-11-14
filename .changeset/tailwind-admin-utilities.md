---
"release": minor
---

# Web text polish and hover in Tailwind classes

Classes now accept `font-sans`, `font-serif`, `font-mono`, and `font-[var(--name)]` (a family from a theme variable), `tabular-nums` and `normal-nums`, `truncate`, `tracking-*`, and `cursor-*`, and a `hover:` variant that changes colors, opacity, or the cursor while a mouse rests over an element.

They are web polish. In the browser they do what Tailwind does, with `hover:` limited to pointers that can hover, so a tap on a phone never leaves an element stuck in its hover look. Native platforms accept the same class strings and ignore them: text keeps the system face, wrapping, and spacing, and nothing hovers. That is why `hover:` refuses layout utilities such as `hover:p-4`: a hover may change how something looks, never where it is.
