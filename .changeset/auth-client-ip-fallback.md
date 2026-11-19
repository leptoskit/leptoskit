---
"leptoskit_auth": patch
---

# Count a request by its connection without the header

With `client_ip_header` set, a request the header was missing from, or whose header wasn't an address, had no address at all, and the limits kept per address didn't count it. Such a request is now taken to come from its connection's address.
