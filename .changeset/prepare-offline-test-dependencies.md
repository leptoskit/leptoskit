---
"release": patch
---

# Prepare dependencies for offline compile checks

`test:cargo` fetches the locked host dependencies before running tests. Restored build outputs alone do not provide every source archive needed by the offline compile-fail fixtures. A preparation failure now stops the gate before the test suites start.
