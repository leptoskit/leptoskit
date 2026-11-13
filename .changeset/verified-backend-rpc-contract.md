---
"release": patch
---

# Exercise nonempty RPC image contracts

The dependency-free `verified-backend` acceptance fixture now exports a query and a channel, so a real reproduction exercises a nonempty structural contract instead of an empty one. The acceptance driver derives its pinned baseline from that actual candidate export and rejects self-consistent baselines that remove or change a previously published function or channel. This is packaging and declaration evidence, not endpoint, cloud, provenance, or runtime-attestation proof.
