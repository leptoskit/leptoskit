---
"leptoskit_cloud_worker": patch
---

# A build worker's shared build cache is bounded

A build worker keeps each project's build cache on one volume that every organisation whose builds run on the worker shares. Nothing capped a cache and nothing removed another project's, so one build with a large layer could fill the volume, and every organisation's next build then failed for want of space until someone deleted directories by hand.

Now a cache export larger than `--build-cache-max-mib` is removed once its build has succeeded. Before each build, the caches of other projects that no build has used for `--build-cache-idle-days` are removed, then the least recently used until no more than `--build-cache-max-used-percent` of the volume is taken. A build never removes the cache of the project it builds, and no build's result changes: a build whose cache was removed is only slower next time.
