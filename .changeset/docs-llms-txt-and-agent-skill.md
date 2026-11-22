---
"leptoskit_docs": minor
---

# Generate llms.txt, llms-full.txt, and Markdown chapters

`leptoskit_docs::Site::build` now emits `llms.txt` (structured chapter index), `llms-full.txt` (concatenated single-file reference), and cleaned `.md` chapter copies beside each HTML page. The site packager and Axum router expose `/llms.txt` and `/llms-full.txt` at the domain root as well as under `/docs/`, and `skills/leptoskit` now ships a `references/platforms.md` catalog for coding agents.
