---
"leptoskit_notifications": minor
---

# Choose a collapse key and a push lifetime

A score that changes every few minutes should replace the last push, not add a banner, and a push worth ten minutes should not stay queued at a provider for days. Until now every push collapsed by its notification's id, and the only lifetime was `expires_at`, which also removes the item from the inbox. Three new options fix this, and a send that sets none of them is byte for byte what it was.

- `MessageOptions::collapse_key` is a key of your own, 1 to 32 letters, digits, `-`, and `_` (`MAX_COLLAPSE_KEY_LEN`). It is sent as `apns-collapse-id`, as FCM's `collapse_key` and the Android notification's tag (unless the send sets `android.tag`), and as the Web Push `Topic` header and the banner's tag in the browser.
- `MessageOptions::push_ttl` is how many seconds a provider may hold the push, counted from when the notification becomes visible, at most 2,419,200 (`MAX_PUSH_TTL_SECS`, 28 days). The inbox item is not affected. With `expires_at` too, the push stops at the earlier one. `0` means one attempt that no provider stores and that is never retried.
- `AndroidOptions::raw_channel_id` names an Android channel exactly as the device holds it, for a channel an earlier build of your app created itself, for example under Firebase. `channel_id` still names a channel made with the plugin. A send sets one or the other.

**What changes for an existing app:**

- `AndroidOptions` and `MessageOptions` have new public fields. Code that builds either one by naming every field no longer compiles: end the literal with `..AndroidOptions::default()` or `..MessageOptions::default()`.
- A delivery that waited past the end of its `push_ttl` ends with the reason `expired`, which until now meant only that `expires_at` had passed.
- The receiver service worker reads a new `lktag` key. A browser still running the previous worker ignores it and shows the banner under the notification's id, as before.
- No migration. Both options are stored in the options JSON each notification row already has.
