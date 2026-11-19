---
"leptoskit_cli": patch
---

# Generated servers find `LeptoskitConfig` again

The database and auth server templates read `[rpc]` through `leptoskit::LeptoskitConfig`, which `leptoskit` exports only from `leptoskit::config` and its prelude, so a newly generated project didn't compile. They now name `leptoskit::config::LeptoskitConfig`.
