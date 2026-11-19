---
"leptoskit_cloud_runtime": minor
---

# Harden cloud cluster manifests and fix Day-1 platform blockers

`ClusterProfile` gains `Serialize`/`Deserialize`, `edge_terminated_custom_domains` (for Cloudflare for SaaS custom domains without cert-manager annotations), `tenant_node_selector` and `tenant_tolerations` (to isolate tenant pods onto dedicated tenant pools), and `tmp_size_limit_mib` (bounding `/tmp` `emptyDir` and container/namespace `ephemeral-storage`). `worker_admission_policy` emits a `ValidatingAdmissionPolicy` restricting cluster-worker mutations to `lk-*` tenant namespaces.

The platform generator fixes CloudNativePG superuser access, `max_connections` (300), and `pg_hba`; renames the KEDA HTTP `HelmChart` to `keda-add-ons-http`; re-enables `metrics-server` and kubelet metrics scraping for CPU HPAs; adds k3s `secrets-encryption`; fixes Cloudflare Tunnel catch-all routing for custom domains and adds an `apps-http01` HTTP-01 `ClusterIssuer` for `cert-manager` mode; keeps Zot cluster-local by default; hardens `cloudflared` and `platform-ingress` allowlist policies; allows ICMP in `nftables`; supports per-node `isolation_platform` overrides and hot/cold S3 backup splits; and fixes hybrid `growth.toml` disk sizing and `advertise-address`.
