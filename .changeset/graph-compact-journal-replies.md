---
"release": patch
---

# Keep large graph inventories out of SSH replies

Graph release preparation, activation and status return compact journal summaries. Complete recovery inventories stay on the host, so publishing another large static tree no longer overflows the bounded transport response and blocks retries.
