---
"leptoskit_plugin_picture_in_picture": patch
"leptoskit_plugin_camera": patch
"leptoskit_plugin_device_keys": patch
"leptoskit_plugin_calendar": patch
"leptoskit_plugin_contacts": patch
"leptoskit_plugin_print": patch
"leptoskit_plugin_sms": patch
"leptoskit_plugin_mail_composer": patch
"leptoskit_plugin_web_browser": patch
"leptoskit_plugin_media_library": patch
---

Android plugins compile against the public SDK again. Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read.
