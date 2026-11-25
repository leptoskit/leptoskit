---
"leptoskit_cloud": patch
---

# Build logs are charged what they keep, and bounded in chunks

A build's log is charged to the build's cap (`ShippingSettings::build_log_cap`) and to its organisation's (`organization_log_cap`) as its chunks are stored, not as the worker sent their lines. A chunk of blank lines used to be kept and charged nothing, so neither cap ever engaged; it now counts like any other. A job's log also has at most `MAX_BUILD_LOG_CHUNKS` (4,096) chunks over all its attempts, whatever their bytes, and a chunk past that is refused with `409 log_full`.
