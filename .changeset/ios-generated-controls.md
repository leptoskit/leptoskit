---
"release": patch
---

# Render common controls in generated iOS apps

Generated iOS apps now render switches, checkboxes, radio options, sliders, progress bars, and activity indicators. Preserve their state through binary transactions, send toggle and value-change proposals to the guest, and expose native selection and value accessibility.

Tapping outside an input dismisses the keyboard while preserving button actions. Return ends editing and submits; guest-controlled focus can move to another field. Scroll views support interactive keyboard dismissal.
