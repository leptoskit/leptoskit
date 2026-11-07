---
"release": patch
---

# Check the complete published RPC contract

`rpc check` now detects changes to existing argument types and requiredness, referenced schemas, channel payloads and join keys, live paths, and error encodings. Current exclusions cannot erase operations from the previous published contract. Missing references and ambiguous operation names require review. The shared `ApiSpec::breaking_changes` comparison is available without the code generators feature for artifact tooling; it checks declarations and does not claim runtime or authorization compatibility.
