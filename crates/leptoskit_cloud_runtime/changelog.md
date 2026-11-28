# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Harden cloud manifests and fix Day-1 platform blockers

_Owner:_ Ifiok Jr. · _Introduced in:_ [29e916b](https://github.com/leptoskit/leptoskit/commit/29e916be038faad465ec62d3fc0e0dff905cfd56)

`ClusterProfile` gains `Serialize`/`Deserialize`, `edge_terminated_custom_domains` (for Cloudflare for SaaS custom domains without cert-manager annotations), `tenant_node_selector` and `tenant_tolerations` (to isolate tenant pods onto dedicated tenant pools), and `tmp_size_limit_mib` (bounding `/tmp` `emptyDir` and container/namespace `ephemeral-storage`). `worker_admission_policy` emits a `ValidatingAdmissionPolicy` restricting cluster-worker mutations to `lk-*` tenant namespaces.

The platform generator fixes CloudNativePG superuser access, `max_connections` (300), and `pg_hba`; renames the KEDA HTTP `HelmChart` to `keda-add-ons-http`; re-enables `metrics-server` and kubelet metrics scraping for CPU HPAs; adds k3s `secrets-encryption`; fixes Cloudflare Tunnel catch-all routing for custom domains and adds an `apps-http01` HTTP-01 `ClusterIssuer` for `cert-manager` mode; keeps Zot cluster-local by default; hardens `cloudflared` and `platform-ingress` allowlist policies; allows ICMP in `nftables`; supports per-node `isolation_platform` overrides and hot/cold S3 backup splits; and fixes hybrid `growth.toml` disk sizing and `advertise-address`.

- **Apply tenant releases through Kubernetes with bounded rollout waits, observed status, logs, safe pruning and namespace deletion.** Align sleep routing and cluster profiles with the platform add-ons and generate the worker's permissions. _Owner:_ Ifiok Jr. · _Introduced in:_ [ce06091](https://github.com/leptoskit/leptoskit/commit/ce06091df4bfe19985bb279678f2776aea1744dd)
- **LeptosKit Cloud's runtime crate: a release as data, and the Kubernetes objects that carry it.** `ReleaseSpec` is the whole desired state of one environment — the app and its services with exact image digests, ports, health paths, environment and secret variables, resources, scale, and volumes; the routes; and the policy — with typed validation that refuses a spec no runtime could honor, and a namespace hashed from the organisation, project, and environment ids so no customer-chosen name ever reaches Kubernetes. The `Runtime` trait is the adapter boundary (`apply`, `status`, `logs`, `destroy`), usable as `dyn Runtime`, with a recording fake behind the `testing` feature that validates specs and scripts ready, never-ready, and unavailable rollouts. The `kubernetes` feature builds the release's manifests deterministically: the namespace under the restricted Pod Security profile, quota and limit range, default-deny network policies with the release's doors only, a token-less service account, hardened Deployments, Services, KEDA's HTTP scaled object or a horizontal autoscaler, disruption budgets, and Ingresses with wildcard or cert-manager certificates — refusing, before anything is applied, sleep without the HTTP add-on, cookie stripping a plain Ingress cannot do, a secret with no value, and an issued certificate with no issuer. _Owner:_ Ifiok Jr. · _Introduced in:_ [a66ec54](https://github.com/leptoskit/leptoskit/commit/a66ec5415deb156d6ac5e38f2c3aa29c4e172003)
- **A release can route to a service the platform runs.** `ReleaseSpec.platform` names services the platform runs outside the environment, such as its cluster's shared notifications hub. A route may name one instead of a process. The manifests give each a bridge in the environment's namespace, an `ExternalName` Service with the service's port, route to it, and open the namespace's egress to that service's namespace on that port and nothing more of it. A spec without platform services serializes as before, and one written before they existed reads with none. _Owner:_ Ifiok Jr. · _Introduced in:_ [8af1a5d](https://github.com/leptoskit/leptoskit/commit/8af1a5d4bf541ce8291c93badfa7bc5e179be046)
- **Reserve one surge pod for every process that rolls, and expose `Quota::for_processes` so control-plane quotas match runtime validation.** Processes with persistent volumes use Recreate and need no surge capacity. _Owner:_ Ifiok Jr. · _Introduced in:_ [ce06091](https://github.com/leptoskit/leptoskit/commit/ce06091df4bfe19985bb279678f2776aea1744dd)

### Fixes

#### Repair the failures blocking the push gate

_Owner:_ Ifiok Jr. · _Introduced in:_ [81054a0](https://github.com/leptoskit/leptoskit/commit/81054a09cf174adcc8509131104b170f42054055)

Test-only fixes: a missing semicolon in a `leptoskit_cloud_runtime` test that clippy rejects with `semicolon_if_nothing_returned`, the `pushes_paused` field in a `leptoskit_cli` cloud test's notifications fixture, the `lk cloud notifications` help and output snapshots that `leptoskit_cli` was missing or held stale, and the migration count a `leptoskit_notifications` restart test pins, which the push acceptance ledger's migration left behind.

The demo's generation fixture also builds its git repository outside the one a `pre-push` hook names, so the hook's own tests no longer stage into the repository being pushed. Its pinned workspace schema snapshot is refreshed for the notification tenant, lease, and push-acceptance tables that landed after it was pinned.
