---
"leptoskit_cli": minor
"leptoskit_config": minor
---

# Meta Horizon Store publishing for Meta Quest

Add `[publish.quest]` configuration and `leptoskit publish quest` to build a release-signed Meta Quest APK and upload it to Meta Horizon Store release channels (`alpha`, `beta`, `rc`, `store`, or subchannels) through Meta's `ovr-platform-util upload-quest-build`.

- Configures `app_id`, `channel`, `age_group` (`teens_and_adults`, `mixed_ages`, `children`), `draft`, and optional `obb`, `assets_dir`, and `debug_symbols_dir` in `[publish.quest]` with updated JSON schemas.
- Accepts `LEPTOSKIT_PUBLISH_QUEST_APP_SECRET` (preferred) or `LEPTOSKIT_PUBLISH_QUEST_TOKEN`, supports `LEPTOSKIT_OVR_PLATFORM_UTIL` overrides, redacts secrets from tool errors, and rejects x86-only ABIs before compiling.
- Defaults to `--draft` uploads until `--submit` is passed with an `age_group`, prints the remaining Meta Horizon Developer Dashboard steps (listing copy/art, Data Use Checkup, content rating, and VRC/Content review submission), and wires `quest` into the generated GitHub Actions `release.yml` workflow.
