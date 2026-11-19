---
"leptoskit_native": minor
"leptoskit": minor
---

# Open links in a new tab, save them, or open them natively

A `Pressable` link can now say where it opens and whether it is a download.

```rust
Pressable("Open the admin")
	.href("https://admin.example.com/")
	.target(LinkTarget::NewTab)
	.child(Text("Open the admin ↗"));

Pressable("Export").href("/people/7/export").download("person-7.json");
```

On the web, `LinkTarget::NewTab` renders `target="_blank" rel="noopener noreferrer"` and tells screen reader users the link opens in a new tab. `download(name)` renders `<a href download>`, so the browser saves the file under that name. A name that is not one plain file name is ignored.

Native apps used to ignore `href`. With the new `links` feature, pressing a link that has no `on_press` handler opens an absolute HTTP(S), `mailto:`, `tel:` or `sms:` link outside the app, through `leptoskit_plugin_linking`: directly on macOS, Windows and Linux, and through the host's `linking` plugin in a Wasm guest on iOS and Android. A link with a handler, a relative link and a disabled link behave as before.
