---
"leptoskit_config": patch
"leptoskit_cli": patch
---

# More ergonomic multi-frontend scaffold

`--frontends` alone scaffolds landing, app, and admin. Ports are stable per kind. The product app under `apps/app` uses the workspace name as its Cargo package. Frontend names trim whitespace and accept ASCII case; `ApplicationKind` gains `FromStr`, `Display`, and helpers used by the scaffold. Each frontend app uses the same file-router defaults as a normal `leptoskit new`.
