---
"release": minor
---

# Generate an edge configuration for each public graph host

`lk infra generate` now writes `edge-<host>.json` beside the graph's provider configuration. Static frontends keep their declared path mounts and asset ownership; SSR frontends and public services use their own loopback ports. Missing nested assets cannot fall through to the landing page or another backend.

The generated Caddy configuration preserves query strings, checks origin TLS host identity, and derives forwarded identity from the configured ingress trust boundary. Graph services strip ambient cookies unless explicitly declared `trust = "first-party"`. SSR frontend sessions remain enabled. HTTPS accepts only the recorded Cloudflare ranges as trusted proxies; Tunnel listens on loopback and trusts the local tunnel connector.

Generation preserves operator edits unless `--force` is supplied. Edge installation, certificates, application activation and provider deployment remain separate pending lifecycle work; generating these files does not claim a running deployment.
