---
"leptoskit_plugin_sms": minor
"leptoskit_plugin_mail_composer": minor
"leptoskit_plugin_web_browser": minor
"leptoskit_plugin_linking": patch
---

# SMS, mail composer, and dismissible web browser plugins

Three new Expo-equivalent runtime plugins cover composer and browsing presentation that were still missing:

- **`sms`** queries composer availability and presents the system SMS sheet with optional recipients and body. Completion is the OS callback only, never delivery. Outbound `sms:` opens stay in `linking`.
- **`mail-composer`** queries availability and presents the system mail sheet with to/cc/bcc/subject/body. Attachments are deferred in this MVP. Outbound `mailto:` opens stay in `linking`.
- **`web-browser`** opens HTTP(S) in Safari View Controller / Custom Tabs and answers on dismiss. It is distinct from `web-auth` (no OAuth return address) and from `linking.open` (which does not wait for dismiss).

Each plugin ships guest clients, wire codecs, Apple/Android backends (with unavailable stubs where a platform has no composer or dismissible sheet), docs, and tests. Desktop hosts mark the plugins unavailable via metadata.
