---
"release": patch
---

# Activate services on hosts without public routes

Graph releases can activate executable services on a host with no public routes without supplying Caddy, TLS certificates, a tunnel or an edge account. Public hosts still require independently pinned Caddy. Mixed selections send the edge runtime only to public hosts. Private services retain process readiness, immutable artifact checks, isolated version history and rollback. Changing a host between public and private roles requires a separate migration; existing edge units are never silently disabled.
