---
leptoskit_plugin_cellular: minor
---

# Cellular carrier and radio snapshot

`leptoskit_plugin_cellular` reads carrier name, MCC/MNC, radio generation, and VoIP allowance when the platform exposes them. Unknown, Unavailable, and Restricted stay explicit. iOS uses CoreTelephony, Android uses permission-light TelephonyManager fields, and desktop plus the browser answer Unavailable.
