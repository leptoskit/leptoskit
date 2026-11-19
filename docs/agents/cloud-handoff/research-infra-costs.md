# Infrastructure cost research for a Rust-first managed app cloud

Research date: **2026-11-18**. All prices ex-VAT unless stated. FX assumption: **€1 ≈ $1.17** (taken from a Sept 2026 article; UNVERIFIED against a bank rate).

Conventions:
- **[P]** = read from the provider's own page on 2026-11-18.
- **[S]** = secondary source (blog, aggregator, search snippet); provider page was blocked or did not render.
- **UNVERIFIED** = I could not confirm the number; do not budget on it without checking.
- Numbers marked **(calc)** are my own arithmetic from sourced inputs. Assumptions are stated next to them.

Fetch-safety note: no fetched page asked me to identify anyone, add headers or contact anyone. No searches or fetch URLs contained personal data. No sign-ups, no paid API calls, no subagents.

Blocked or unrendered provider pages (so those figures are [S] or UNVERIFIED): Scaleway pricing (HTTP 403), Vultr pricing (403), Render pricing (tables did not render), Google Cloud Run pricing (page truncated; the fetcher fell back to model memory, so those rates are UNVERIFIED), Azure Container Apps unit rates (rendered as "$-"), Linode/Akamai compute prices (page lists no numbers).

---

## 0. Key findings in one screen

1. **Hetzner got much more expensive in 2026 and the cheap Cloud SKUs are often sold out.** Two rounds: April 1 (+~30%) and June 15 (CX/CAX +30–40%, CPX +144–175%, CCX +113–173% for new orders and rescales). The official Hetzner CX and CAX pages show "Currently unavailable" for every size on 2026-11-18. Dedicated AX servers are still the best raw price per GB of RAM in the EU and include unlimited traffic.
2. **Hetzner Cloud has no nested virtualisation (no `/dev/kvm`)** per third-party reports, so Kata, Firecracker and KubeVirt cannot run there. Hetzner dedicated servers expose KVM. gVisor (systrap) runs anywhere.
3. **No managed Kubernetes at Hetzner** (confirmed absent in several Oct 2026 sources). Free managed control planes exist at DigitalOcean, Vultr, Civo, OVH (free tier), Scaleway (mutualized) and Linode (no per-cluster fee).
4. **Cheapest credible architecture:** self-managed k3s on Hetzner (one dedicated AX box at launch, small AX fleet for growth), CloudNativePG with WAL archived to object storage, Cloudflare in front (free plan + Cloudflare for SaaS custom hostnames, first 100 free then $0.10 each per month), R2 or B2 for objects, your three owned Mac minis for iOS and macOS builds, and Linux builds on an own AX box.
5. **macOS builds are where rented capacity is expensive.** Owned Mac minis cost cents per hour in power. Rented Macs are €0.22/h (Scaleway M4-S, 24 h minimum) to $1.23/h (AWS mac-m4, 24 h minimum). GitHub charges $0.062/min for macOS.
6. **Apple's macOS licence limits leasing and "service bureau" use.** Running third-party customers' iOS builds on your own Macs needs legal review (see "Apple licence restrictions" under Q7).

---

## 1. Recommendation

### 1a. Stage A: launch, fewer than 50 customers, near-zero fixed cost

Two sub-options. Choose by how many customers are real.

**A-lean (0–10 customers): about €20–25 / $24–29 per month fixed**

| Item | Choice | €/month | $/month |
|---|---|---|---|
| Nodes | 2 small VPS, 4 vCPU / 8 GB each, k3s (1 server + 1 agent), e.g. 2× Hetzner CX33 (€8.99 each, if orderable) or 2× OVH VPS-2 ($8.50 each) | 17–18 | 17–21 |
| Postgres | CloudNativePG, one shared instance on a node, WAL to R2 or B2 | 0 | 0 |
| Isolation | gVisor (systrap) RuntimeClass + NetworkPolicy + Pod Security restricted | 0 | 0 |
| Edge / DNS / TLS | Cloudflare Free + Let's Encrypt wildcard via DNS-01; custom domains via Cloudflare for SaaS (first 100 free) | 0 | 0 |
| Object storage | R2 free tier (10 GB-month, 1M Class A, 10M Class B) then $0.015/GB | 0–2 | 0–2 |
| Registry | GHCR (free today, GitHub promises notice before charging) or zot on R2 | 0 | 0 |
| Linux builds | GitHub Actions free minutes at launch, or off-peak on the same nodes | 0 | 0 |
| macOS builds | Your 3 owned Mac minis (power and connection only) | ~0 marginal | ~0 marginal |
| **Total** | | **≈ €20** | **≈ $24** |

Capacity of A-lean (calc, section 4): about 16 GB total, so roughly 25 *active* 128 MB apps under gVisor, hundreds if most sleep. No HA. Hetzner Cloud CX/CAX stock is the risk; OVH VPS or Contabo are fall-backs. Contabo's Cloud VPS is cheapest per GB but is shared and has fair-use egress; do not run untrusted tenants there.

**A-standard (10–50 customers): about €60–65 / $70–76 per month fixed**

| Item | Choice | €/month | $/month |
|---|---|---|---|
| Tenant + control node | **1× Hetzner AX41** (Ryzen 5 3600, 6C/12T, 64 GB, 2× NVMe, unlimited 1 Gbps traffic, €0 setup) or the cheapest Server Auction box (a 1 Oct 2026 snapshot saw €57 for the same CPU class) | 57–59 | 67–69 |
| Extras | Hetzner Object Storage base €6.49 only if you want S3 in-region; otherwise R2 free tier | 0–6.49 | 0–7.6 |
| Edge | Cloudflare Free + Cloudflare for SaaS | 0 | 0 |
| **Total** | | **≈ €59–66** | **≈ $69–77** |

Run k3s on the box with two RuntimeClasses: `gvisor` for tenant pods and `runc` for platform pods. Because it is bare metal, `/dev/kvm` exists, so gVisor can use the KVM platform and Kata/Cloud Hypervisor is available later without moving. Single box means a single failure domain; keep daily backups to B2/R2 and a rebuild runbook. Add a €6–9 VPS as a cold standby and external uptime probe once you have paying customers.

### 1b. Stage B: growth to thousands of apps: roughly €1,100 / $1,300 per month for the first ~3,000–5,000 mostly sleeping apps (calc)

| Role | Hardware | Qty | €/month each | €/month total |
|---|---|---|---|---|
| Tenant workers (Kata/Cloud Hypervisor or gVisor-KVM) | Hetzner AX102 (16C/32T, 128 GB DDR5 ECC, from €159, setup €129) | 3 | 159 | 477 |
| Postgres nodes (CloudNativePG primary, replica, witness/spare) | Hetzner AX41 (€59) | 3 | 59 | 177 |
| Build box (Rust + sccache + registry cache on NVMe) | Hetzner AX102 | 1 | 159 | 159 |
| Control plane (k3s HA, etcd) | 3× small cloud VM (CX23 €5.99 or CAX11 €6.49) | 3 | ~6 | ~18–20 |
| Load balancing | 2× Hetzner LB11 (€7.49 from third-party calculator) or Cloudflare in front plus own HAProxy | 2 | 7.49 | 15 |
| Object storage (≈10 TB) | Hetzner Object Storage: €6.49 for the first TB + ~€6.35/TB after (€0.0087 per TB-hour) | 1 | | ~65 |
| Custom hostnames | Cloudflare for SaaS: 1,000 hostnames ≈ $90 ($0.10 × 900) | | | ~77 |
| Monitoring, backups, misc | | | | ~50–100 |
| **Total** | | | | **≈ €1,040–1,100 ≈ $1,220–1,290** |

- Scale-out is manual: Cluster Autoscaler cannot rent dedicated servers (Robot orders are not elastic). Use Hetzner Cloud VMs through hetzner-k3s or kube-hetzner autoscaling only for gVisor-class burst (no KVM on cloud VMs), and only while CX/CAX stock exists.
- Ubicloud (open source, runs on Hetzner bare metal, Cloud Hypervisor based) is a useful reference for doing KVM-based multi-tenant VMs and Postgres on Hetzner; its managed Postgres is listed at $19.54/month (hobby-1) [P].
- Alternative with no servers to operate: Cloudflare Containers for sleepy hobby apps (section 4), at the cost of higher per-hour compute and platform lock-in.

### 1c. Why not the alternatives

- **Hyperscalers** (Fargate/Cloud Run/Container Apps): 5–20× the cost of amortised bare metal for always-on apps; good for reference pricing only.
- **Fly.io as the substrate:** $2.19/month for a shared-cpu-1x 256 MB machine, stopped machines cost only $0.15/GB rootfs, and it is Firecracker-isolated, but Managed Postgres starts at $38/month and you would be reselling a competitor with thin margin.
- **Hetzner Cloud as the main substrate:** cheapest per vCPU but frequently unavailable, no KVM, 144–175% rises on CPX in June. Fine for control plane and burst.
- **Neon / Supabase for tenant databases:** per-project pricing does not give the margin you need at thousands of tiny databases (section 6).

---

## 2. Unit economics

Assumptions (all calc; change them and the numbers move):
- Stage A-standard box: AX41 at **€59/month, 64 GB, 12 threads**. Reserve 8 GB for OS and k3s and 8 GB for shared Postgres, leaving ~48 GB for tenant pods.
- Memory cost basis: **€0.92 per GB-month** (€59 ÷ 64 GB). CPU is oversold 3:1 on 12 threads → 36 vCPU, **€1.64 per vCPU-month**. To avoid double counting I split the box 50/50: **€0.46 per GB-month + €0.82 per vCPU-month**.
- gVisor sandbox overhead ≈ 50 MB per pod (sources range 31–84 MiB; section 3).
- Shared Postgres: a 4 GB shared instance serves ~200 small databases.
- Storage: B2 at $0.00695/GB-month or R2 at $0.015/GB-month.
- "Loaded" cost multiplies raw by ~2.5 to cover HA headroom (2× nodes), monitoring, backups, support and idle fleet capacity.

| Workload | Raw marginal cost / month | Loaded cost / month |
|---|---|---|
| **Idle hobby app** (scale-to-zero, 128 MB image, 5% awake, small Postgres DB, 1 GB storage) | ≈ €0.05–0.10 ($0.06–0.12): compute ~€0.02, DB share ~€0.02, storage + WAL ~€0.02, image cache ~€0.005 | ≈ **$0.15–0.30** |
| **Always-on small production app** (0.5 vCPU / 512 MB, 1 GB Postgres, 10 GB storage, 100 GB transfer) | ≈ €1.3 ($1.50): compute €0.41 + memory €0.23 + sandbox €0.05 = €0.69; Postgres 1 GB ≈ €0.30; storage 10 GB ≈ $0.07 (B2) to $0.15 (R2); transfer €0 on Hetzner dedicated (unlimited), ~$1 if served via Bunny at $0.01/GB; hostname $0–0.10 | ≈ **$3–4** |
| **Linux build minute** (4 vCPU-equivalent) | ≈ €0.0009 ($0.0011) at 50% utilisation of an AX42 (€79, 16 threads: €79 ÷ 21,600 busy minutes = €0.00366 per box-minute, ÷4); ≈ €0.0018 at 25% | ≈ **$0.002–0.003** with cache storage and idle time |
| **macOS build minute** (owned Mac mini) | Power + line only: ≈ $0.0002–0.0005 marginal. Fully amortised mini ($599 list UNVERIFIED over 36 months ≈ $17/month + ~$3 power/line) ≈ $0.0011/min at 40% utilisation, $0.0046/min at 10% | ≈ **$0.001–0.005** owned. Rented Scaleway M4-S €149/month ≈ $0.0099/min at 40% utilisation, ≈ $0.04/min at 10% |

### Suggested price points (gross margin 60–70% = price ≈ 2.5–3.3× cost)

| Product | Our loaded cost | Price for ~65% GM (3×) | Suggested list price | Positioning vs competitors |
|---|---|---|---|---|
| Hobby app (sleeps, shared DB, 1 GB) | $0.15–0.30 | $0.5–1 | **Free tier, one app, sleeps after 10 min, hard caps** (marketing cost: CAC) or $5 flat for 3 apps | Serverpod Starter $5; Railway Hobby $5 with $5 credit; Heroku Eco $5; Render Hobby $0 |
| Always-on small prod app (0.5 vCPU/512 MB, 1 GB DB, 10 GB, 100 GB) | $3–4 | $9–12 | **$12–15 per month all-in** (≈ 75–80% GM) | Render Starter $7 + Postgres $19 = $26+; Railway ≈ $15 compute + volume + $5 plan; Fly ≈ $3.69 + $38 Postgres; Serverpod Cloud M podlet ≈ $29 + S database ≈ $31 |
| Linux build minute (4 vCPU) | $0.002–0.003 | $0.006–0.009 | **$0.005 per minute**, e.g. 1,000 minutes included on paid plans | GitHub $0.006 (2-core), Blacksmith $0.004, WarpBuild $0.004, Ubicloud $0.00125 (2 vCPU) |
| macOS build minute (owned minis) | $0.001–0.005 | $0.003–0.015 | **$0.04 per minute** (≈ 88–97% GM owned, positive down to ~12% utilisation of a rented M4-S) | GitHub $0.062, Blacksmith/WarpBuild $0.08, Codemagic $0.114 (M4) / $0.095 (M2), EAS $2 per iOS build |

Note the owner's instruction is "minimise our costs", not "undercut". Competitor prices are far above the raw cost of owned hardware, so price against the market and keep the margin.

---

## 3. Detailed tables per question

### Q1. Compute for tenant workloads

All EUR figures ex-VAT. "Per vCPU" and "per GB" are the plan price divided by that count, so the two columns are not additive.

#### Hetzner Cloud [P] hetzner.com/cloud/{regular-performance,general-purpose,cost-optimized}, read 2026-11-18

Page prices appear to include the €0.50 IPv4 (CX23 shows €5.99 on Hetzner's page vs €5.49 ex-IPv4 on third-party trackers; CPX22 NBG €19.99 vs €19.49). Billing is hourly with a monthly cap; powered-off servers still bill. Locations: Falkenstein, Nuremberg, Helsinki (EU), Hillsboro and Ashburn (US), Singapore.

| Plan | vCPU / RAM / disk | €/month | €/vCPU | €/GB | Included traffic | Note |
|---|---|---|---|---|---|---|
| CX23 (Intel/AMD shared) | 2 / 4 GB / 40 GB | 5.99 | 3.00 | 1.50 | 20 TB EU | "Currently unavailable" |
| CAX11 (Ampere ARM) | 2 / 4 / 40 | 6.49 | 3.25 | 1.62 | 20 TB EU | unavailable |
| CX33 | 4 / 8 / 80 | 8.99 | 2.25 | 1.12 | 20 TB | unavailable |
| CAX21 | 4 / 8 / 80 | 10.99 | 2.75 | 1.37 | 20 TB | unavailable |
| CX43 | 8 / 16 / 160 | 16.49 | 2.06 | 1.03 | 20 TB | unavailable |
| CAX31 | 8 / 16 / 160 | 21.49 | 2.69 | 1.34 | 20 TB | unavailable |
| CX53 | 16 / 32 / 320 | 29.99 | 1.87 | 0.94 | 20 TB | unavailable |
| CAX41 | 16 / 32 / 320 | 41.49 | 2.59 | 1.30 | 20 TB | unavailable |
| CPX22 (AMD shared) | 2 / 4 / 80 | 19.99 NBG, 16.99 HEL | 10.0 | 5.0 | 20 TB | CPX listed but some "unavailable" |
| CPX32 | 4 / 8 / 160 | 35.99 NBG, 30.49 HEL | 9.0 | 4.5 | 20 TB | |
| CPX42 | 8 / 16 / 320 | 69.99 NBG, 59.49 HEL | 8.75 | 4.4 | 20 TB | |
| CCX13 (dedicated vCPU) | 2 / 8 / 80 | 36.99 HEL, 43.49 NBG | 18.5 | 4.6 | 20 TB | |
| CCX23 | 4 / 16 / 160 | 73.49 HEL | 18.4 | 4.6 | 20 TB | |
| CCX33 | 8 / 32 / 240 | 117.99 HEL | 14.7 | 3.7 | 30 TB | |
| CCX43 | 16 / 64 / 360 | 234.99 HEL | 14.7 | 3.7 | 40 TB | |

- US: CPX11 2/2 GB €17.99, CPX21 3/4 GB €32.49, CPX31 4/8 GB €62.99; included traffic only 1–5 TB; no CX/CAX in US.
- Singapore: CPX02 €7.99 (1 vCPU/1 GB), 0.5–5 TB included.
- Overage: **€1.00/TB** EU and US, **€7.40/TB** Singapore [S: costgoat, updated 2026-10-20 and bex.co].
- IPv4 €0.50/month even unattached; IPv6 free [S: trackvps].
- Backups 20% of server price; snapshots €0.0143/GB-month; volumes €0.0572/GB-month; LB11 €7.49/month [S: costgoat 2026-10-20].
- Price history [S: trackvps, byteiota, bex.co]: CX23 €2.99 → €3.99 (1 Apr) → €5.49 (15 Jun); CPX22 €5.99 → €7.99 → €19.49; CCX13 €11.99 → €15.99 → €42.99. Existing servers keep old rates unless resized.
- Stock: one source says new servers have been restricted since 26 Jun 2026; the official CX/CAX page showed every size "Currently unavailable" on 2026-11-18.

#### Hetzner dedicated [P] hetzner.com/dedicated-rootserver/matrix-ax/ and hetzner.com/sb/, read 2026-11-18

| Model | CPU | RAM | Disk | €/month ("from"; max configured) | Setup | €/core | €/thread | €/GB (base RAM) |
|---|---|---|---|---|---|---|---|---|
| AX41 | Ryzen 5 3600, 6C/12T | 64 GB DDR4 | 2× 512 GB NVMe | 59.00 (max 59.00) | €0 | 9.83 | 4.92 | 0.92 |
| AX42 | Ryzen 7 PRO 8700GE, 8C/16T | 64 GB DDR5 ECC | 2× 1.92 TB+ NVMe | 79.00 (max 99.00) | €49 | 9.88 | 4.94 | 1.23 |
| AX102 | Ryzen 9 7950X3D, 16C/32T | 128 GB DDR5 ECC (inferred base) | 2× 1.92 TB+ NVMe | 159.00 (max 259.00) | €129 | 9.94 | 4.97 | 1.24 |
| AX162 | EPYC 9454P, 48 cores | 128 GB–512 GB DDR5 ECC reg | 2× 1.92 TB+ NVMe | 319.00 (max 614.00) | €304 | 6.65 | ~3.3 (if 96T) | 2.49 |

- The page maps RAM and disk options per tier by list order, so RAM/disk per model above is my inference.
- Traffic: **unlimited at 1 Gbit/s** (policy does not apply with the 10G uplink add-on). One IPv4 + IPv6 /64 included on the auction page.
- Server Auction: page shows only a "from €78.70/month" headline. A 2026-10-01 snapshot [S: ployz/trackvps] saw 170 listings, **cheapest €57** (Ryzen 5 3600, 64 GB, 4× 512 GB NVMe, no setup fee), median €86. One forum post lists Ryzen 7 3700X 64 GB ECC for €58.70 with a €125 transfer fee. Prices change hourly.
- One article teaser claims AX102 went from €124 to €454 in June; the official page shows "from €159". Treat that claim as UNVERIFIED.
- Hetzner dedicated exposes `/dev/kvm` [S: a 2026 bare-metal roundup and the k7 project README], occasionally disabled in firmware (a support ticket fixes it).

#### OVHcloud

| Product | Spec | Price | Source |
|---|---|---|---|
| VPS-1 | 2 vCore / 4 GB / 40 GB NVMe, 500 Mbps, unlimited traffic | from $4.54/month | [P] ovhcloud.com/en/vps/, shown in USD, tax basis unstated |
| VPS-2 | 4 / 8 / 75 GB, 1 Gbps | from $8.50 ($2.13/vCore, $1.06/GB) | [P] |
| VPS-3 | 6 / 12 / 100 GB, 2 Gbps | from $12.32 | [P] |
| VPS-4 | 8 / 24 / 200 GB, 3 Gbps | from $23.37 | [P] |
| So you Start SYS-GAME-1 | Ryzen 5 3600X, 6C, 32 GB, 2× 512 GB NVMe | $55.30/month, setup = 1 month [S: stackrates, 2026-11-06] | [S] |
| Kimsufi KS-A | i7-6700k, 32 GB, 480 GB, 100 Mbps | $61/month, setup $61 [S: stale page] | [S] |
| Advance-1 (2026) | | €159.99 after 11 Aug 2026 rise from €104.99 [S] | [S] |

- Regions for VPS: "many global locations", not listed per plan on the page. Local Zones lack block storage, load balancer and managed DB.
- Public Cloud instance prices were not fetched: UNVERIFIED. Kimsufi/So you Start/Rise were not part of the Aug 2026 rise [S].

#### Scaleway (official pricing page returned 403; figures from a search tool quoting the official page and aggregators) [S]

- DEV1-S €6.55, DEV1-M €14.74, DEV1-L €31.27 per month; PLAY2-PICO (1 vCPU/2 GB) €10.42; PLAY2-NANO (2/4) €20.10; PLAY2-MICRO (4/8) €40.20; PRO2-XXS (2/8) €40.95; PRO2-XS (4/16) €81.90. COPARM1 (Ampere) ≈ €31 / €63 / €126 for 2C-8G / 4C-16G / 8C-32G (from an older, ~11-month-old table). Block storage ≈ €0.087/GB-month, flexible IPv4 ≈ €2.92/month, egress unmetered. Region: Paris, Amsterdam, Warsaw. UNVERIFIED for current values.

#### Netcup [P] netcup.com/en/server/vps, read 2026-11-18 (12-month term, G12.5 generation introduced 2026-09-22)

| Plan | vCores / RAM / SSD | €/month ex-VAT | €/vCore | €/GB |
|---|---|---|---|---|
| VPS 500 G12.5 | 2 / 4 GB / 64 GB | 8.26 | 4.13 | 2.07 |
| VPS 1000 G12.5 | 4 / 8 / 128 | 14.50 | 3.63 | 1.81 |
| VPS 2000 G12.5 | 8 / 16 / 256 | 26.92 | 3.37 | 1.68 |
| VPS 4000 G12.5 | 12 / 32 / 512 | 45.36 | 3.78 | 1.42 |
| VPS 8000 G12.5 | 16 / 64 / 1 TB | 67.11 | 4.19 | 1.05 |
| RS 1000 G12.5 (dedicated cores) | 4 / 8 / 128 | 18.26 [S] | 4.57 | 2.28 |

Locations: Vienna, Nuremberg, Amsterdam, Manassas (US), Singapore. Traffic "included", volume not stated on the page. Netcup raised prices on 1 May 2026 (+18.5% existing, +24.3% new) and replaced G12 with G12.5 on 22 Sep 2026, with root-server prices up to +108% on a 12-month term [S].

#### Contabo [P] contabo.com/en/vps/, read 2026-11-18

| Plan | vCPU / RAM / disk | €/month at 24-month term (taxes included per page) |
|---|---|---|
| Cloud VPS 4 | 4 / 8 GB / 100 GB SSD, 200 Mbit/s | 4.40 (undiscounted 5.50) |
| Cloud VPS 6 | 6 / 12 GB / 200 GB SSD, 300 Mbit/s | 6.00 |
| Cloud VPS 8 | 8 / 24 / 300 GB, 600 Mbit/s | 11.20 |
| Cloud VPS 16 | 16 / 64 / 500 GB, 1 Gbit/s | 29.60 |

Unlimited traffic, outgoing subject to fair use and may be throttled. Regions: EU, UK, US, Singapore, Japan, India, Australia. Setup fee applies to monthly contracts on VPS 4. Cheapest per GB by far (€0.46–0.55), but shared and oversold; not recommended for untrusted multi-tenant workloads.

#### DigitalOcean [P] digitalocean.com/pricing/droplets, read 2026-11-18

- Basic: $4 (512 MiB, 1 vCPU, 500 GiB transfer), $24 (4 GiB, 2 vCPU, 4,000 GiB), $96 (16 GiB, 8 vCPU, 6,000 GiB) → **$6/GB, $12/vCPU**.
- General Purpose (dedicated CPU): $63 for 2 vCPU/8 GiB; $252 for 8 vCPU/32 GiB. CPU-Optimized $42 for 2 vCPU/4 GiB.
- Billing per second, 60-second minimum, since 2026-01-01. Snapshots $0.06/GB-month. Backups 20% (weekly) or 30% (daily).
- Transfer overage rate not on the page: UNVERIFIED (I recall roughly $0.01/GiB).

#### Vultr [S] (page returned 403; third-party reviews, none from Vultr)

Regular 1 vCPU/1 GB/25 GB/1 TB $5; High Performance 1 vCPU/1 GB/25 GB/2 TB $6; High Frequency 1/1/32 GB NVMe $6; overage $0.01/GB (single source); VKE control plane free. UNVERIFIED against Vultr's own page.

#### Linode/Akamai [P] akamai.com/cloud/pricing

No prices rendered. Page confirms: LKE has no per-cluster fee, HA control plane is a paid add-on, shared/dedicated/high-memory plans exist, newer G8/GPU plans have no bundled transfer. Compute prices: UNVERIFIED.

#### Fly.io [P] docs.fly.io/about/pricing, read 2026-11-18 (cheapest regions iad/ewr; others cost more)

| Item | Price |
|---|---|
| shared-cpu-1x 256 MB | $2.19/month |
| shared-cpu-1x 512 MB | $3.69 |
| shared-cpu-1x 1 GB | $6.70 |
| shared-cpu-2x 2 GB | $13.39 |
| performance-1x 2 GB | $33.00 |
| Extra RAM | $6.00/GB/month |
| Stopped machine rootfs | $0.15/GB/month |
| Volumes | $0.15/GB/month; snapshots $0.08/GB-month (first 10 GB free) |
| Egress | $0.02/GB NA/EU; $0.04 APAC/Oceania/S. America; $0.12 Africa/India; inbound free |
| Dedicated IPv4 | $2/month; static egress IP $0.005/hour |
| Managed Postgres | $38/month (Basic) to $1,922; storage $0.28/GB-month |
| Free allowance | none for new orgs; personal bill under $5 waived; $1 trial credit |

Isolation: Firecracker microVMs [S: fly.io/learn/firecracker-vm].

#### AWS (reference) [P] aws.amazon.com/fargate/pricing, us-east-1, converted from per-second rates

- Fargate Linux x86: $0.0405/vCPU-h ($29.6/vCPU-month), $0.00445/GB-h ($3.25/GB-month). Linux ARM (Graviton): $0.0324/vCPU-h ($23.7/month), $0.00356/GB-h ($2.60/month). Ephemeral storage beyond 20 GB: $0.000111/GB-h. Spot "up to 70% off" (ECS only).
- 0.5 vCPU/512 MB always-on on ARM: ≈ $12.2/month (calc).
- Graviton EC2 prices not fetched: UNVERIFIED.

#### Google Cloud Run

Official page truncated. UNVERIFIED model-memory figures (tier 1): request-based ~$0.000024/vCPU-s, ~$0.0000025/GiB-s, ~$0.40 per million requests; instance-based ~$0.000018/vCPU-s and ~$0.000002/GiB-s; free tier 180,000 vCPU-s, 360,000 GiB-s, 2M requests. Check cloud.google.com/run/pricing before use.

#### Azure Container Apps [P] azure.microsoft.com/en-us/pricing/details/container-apps/

Consumption: per-second active vs reduced idle rate, zero when scaled to zero; free grant per subscription per month: 180,000 vCPU-s, 360,000 GiB-s, 2M requests. Unit rates did not render: UNVERIFIED.

#### Cloudflare Containers and Workers [P] developers.cloudflare.com/containers/pricing and /workers/platform/pricing, read 2026-11-18

- Included in the Workers Paid plan ($5/month minimum). Includes 25 GiB-hours memory, 375 vCPU-minutes, 200 GB-hours disk per month.
- Overage: memory $0.0000025/GiB-s (= $6.48/GiB-month always-on, 30 days), CPU $0.000020/vCPU-s (= $51.84/vCPU-month at 100% active; CPU is billed on active use only), disk $0.00000007/GB-s.
- Instance types: lite 1/16 vCPU, 256 MiB, 2 GB; basic 1/4 vCPU, 1 GiB, 4 GB; standard-1 1/2 vCPU, 4 GiB, 8 GB; standard-2 1 vCPU, 6 GiB, 12 GB; standard-3 2 vCPU, 8 GiB; standard-4 4 vCPU, 12 GiB.
- Billing starts when a request arrives or the container is started and stops when it sleeps → scale-to-zero is built in.
- Egress: $0.025/GB NA/Europe (1 TB included), $0.05 Oceania/Korea/Taiwan, $0.04 elsewhere (500 GB included).
- Each container also needs a Durable Object and Worker request charges.
- An always-on lite container ≈ $1.64 memory + $0.37 disk ≈ $2/month (calc); basic ≈ $7.3/month plus CPU. Good for sleepers, poor for always-on.
- Workers: Free plan 100,000 requests/day, 10 ms CPU per invocation; Paid $5 minimum includes 10M requests and 30M CPU-ms, then $0.30/M requests and $0.02/M CPU-ms. No egress charge.

#### Provider terms on reselling and untrusted code

| Provider | Finding | Source |
|---|---|---|
| Hetzner | General terms §7.1 explicitly allow granting third parties use of ordered services; you stay the sole contractual partner and fully liable. Contract transfer needs consent. Crypto mining prohibited; DDoS and open mail relays prohibited; spam prohibited; Hetzner may lock access without reviewing content. Product-specific "Cloud and vServer" policies not read. A reseller reports repeated friction over customers' copyright content. | [P] hetzner.com/terms-and-conditions; [S] webhostingtalk, lowendtalk |
| Cloudflare (self-serve) | §2.2.1(a): you may not "rent, lease, loan, export, or sell access to the Services to any third party, or sign up for the Services on behalf of a third party". Do not give customers Cloudflare accounts or tokens. Cloudflare for SaaS (custom hostnames) is the sanctioned route for serving customers' domains through your zone. Service-specific terms (CDN non-HTML limits, R2) not read: UNVERIFIED. | [P] cloudflare.com/terms |
| Contabo, OVH, Netcup, Scaleway, DO, Vultr | Not reviewed. Check acceptable-use and reseller clauses before signing. | UNVERIFIED |
| Apple (macOS) | See "Apple licence restrictions" under Q7. | [S] Apple SLA PDFs |

### Q2. Kubernetes

#### Managed control planes

| Provider | Product | Control plane price | Autoscaler | Source |
|---|---|---|---|---|
| Hetzner | none | no managed K8s; no launch announced (several Oct 2026 sources) | n/a | [S] qovery, bitdoze, ankra |
| DigitalOcean | DOKS | **Free**; HA control plane **$40/month**; workers from $12/month (Basic), LB $12/month | free autoscaler | [P] digitalocean.com/pricing/kubernetes |
| Civo | Kubernetes | **Free**; nodes $5.43/month (1 vCPU/1 GB) to $21.73 (2 vCPU/4 GB) to $43.45 (4/8); **egress and ingress free**; LB $10.86 | not stated: UNVERIFIED | [P] civo.com/pricing |
| Scaleway | Kapsule | Mutualized: free (≤150 nodes, 1 replica). Dedicated-4 €80.30/month, Dedicated-8 €131.40, Dedicated-16 €255.50 (30-day commitment) | not verified | [S] quotes scaleway.com pricing |
| OVHcloud | MKS | Free plan (99.5% SLO, ≤100 nodes, etcd 400 MB); Standard **$0.099/cluster-hour (≈$71/month)**, 99.99% on 3-AZ | node-pool autoscaling in both plans | [P] ovhcloud.com/en/public-cloud/kubernetes |
| Linode/Akamai | LKE | No per-cluster fee; HA is a paid add-on (price not on page) | UNVERIFIED | [P] akamai.com/cloud/pricing |
| Vultr | VKE | Free [S] | UNVERIFIED | [S] |
| Ubicloud | K8s | 1 control plane + 1 worker (2 vCPU) $71.82/month; 3 CP + 3 workers $215.46/month | n/a | [P] ubicloud.com/docs/about/pricing |
| AWS EKS | | $0.10/hour ≈ $73/month | Karpenter/CA | [S] qovery |
| GKE | | $0.10/hour with $74.40 monthly credit | built-in | [S] |
| AKS | | Free tier; Standard $0.10/hour; Premium $0.60/hour | built-in | [S] |
| Exoscale | SKS | Starter free; Pro paid (price not given); Karpenter on Pro | | [S] |
| IONOS | | Free control plane | | [S] |
| StackIt | SKE | ≈ €0.0996/cluster-hour | | [S] |

#### Self-managed on Hetzner

| Tool | What it gives | Autoscaler | Source |
|---|---|---|---|
| hetzner-k3s (MIT) | One `cluster.yaml` builds k3s; installs Hetzner CCM, CSI, System Upgrade Controller and Cluster Autoscaler | Yes | [S] ankra.ai 2026-10 |
| kube-hetzner (MIT Terraform module) | k3s or RKE2 on openSUSE Leap Micro, CA node pools, CSI | Yes | [S] |
| Cluster API Provider Hetzner (CAPH, Apache-2.0, by Syself) | Cluster API on cloud and **bare metal** | via Cluster API | [S] |
| Talos + Omni | Immutable OS; Omni free on Hobby plan; production self-host needs a commercial licence | | [S] |
| Syself Autopilot, Cloudfleet, Edka, Ankra | Vendor-managed control planes on your Hetzner account; prices not published (Ankra: first 30 worker vCPUs free) | CA/worker groups | [S] |

- Hetzner officially maintains the cloud controller manager and CSI driver [S: qovery].
- Cluster Autoscaler works with Hetzner **Cloud** VMs, not with Robot dedicated servers. Dedicated servers are provisioned manually (Robot), with setup fees and monthly commitment.

#### Cheapest production-credible multi-node clusters (calc)

| Option | Monthly cost | Notes |
|---|---|---|
| hetzner-k3s: 3× CX23 control plane (€17.97) + 3× CX33 workers (€26.97) | ≈ **€45 / $53** | If CX stock exists. 12 vCPU, 28 GB total. Add LB11 €7.49 |
| Single AX41, single-node k3s | €59 / $69 | 64 GB, no HA; best cost per GB; KVM available |
| Civo: 3× Medium (2 vCPU/4 GB) | $65 | free control plane, free egress, but managed DB is $43/month |
| DOKS: 3× $24 Basic (2 vCPU/4 GiB) + HA $40 | $112 | |
| OVH MKS Standard + 3× VPS/Public Cloud nodes | ≈ $71 + nodes | node prices not fetched |
| Ubicloud 3 CP + 3 workers | $215.46 | all-in networking |

### Q3. Multi-tenant isolation for untrusted code

Context: tenants write arbitrary Rust and ship a binary, so assume a hostile tenant, not just a noisy one.

| Mechanism | Needs KVM? | Works on Hetzner Cloud / cheap VPS? | Overhead (secondary sources) | Isolation strength |
|---|---|---|---|---|
| Namespaces + NetworkPolicy + Pod Security + seccomp + resource quotas | No | Yes | ~0 | Shared kernel; one kernel CVE crosses tenants |
| Capsule / vcluster (tenancy control planes) | No | Yes | vcluster adds a virtual API server per tenant (small) | Control-plane isolation only; same data-plane kernel unless combined with a sandbox runtime |
| gVisor (runsc) | **No**: systrap platform default since 2023; KVM platform optional and best on bare metal (gVisor docs recommend systrap inside VMs) | **Yes** | ~20–50 MB RAM per pod (measured 30.9 MiB idle; other sources 50–84 MiB); startup ≈ 0.95 s pod-Ready vs 0.84 s runc in one benchmark; syscall-heavy loops 2.5–5× slower, bulk TCP −10–20%, I/O-heavy workloads 10–30% slower | Reduced host-kernel attack surface (about 70 host syscalls); rated for moderately untrusted multi-tenant |
| Kata Containers (QEMU / Cloud Hypervisor / Firecracker) | **Yes** | **No** on Hetzner Cloud (no `/dev/kvm`); yes on dedicated | 100–180 MB RAM per pod (measured 179.9 MiB with QEMU; Cloud Hypervisor RuntimeClass example sets 130Mi overhead); startup 0.2–0.5 s ideal, 1–5 s measured on nested hosts; 8–12% steady CPU overhead; I/O near native | Own guest kernel; rated for adversarial multi-tenant |
| Firecracker (+ firecracker-containerd, Fly-style) | **Yes** | **No** on Hetzner Cloud | ~5 MB VMM + guest kernel; no GPU; resume from snapshot a few hundred ms | Strong; used by Fly.io, AWS Lambda |
| Cloud Hypervisor | **Yes** | **No** on Hetzner Cloud | Similar to Firecracker with broader device support | Strong; used by Northflank and Koyeb |
| KubeVirt (full VM per tenant) | **Yes** | **No** on Hetzner Cloud | A guest OS per VM (hundreds of MB) | Strongest; heavy for 64 MB apps |

Facts on KVM availability:
- Hetzner Cloud: third-party reports of Ubuntu 24.04 on AMD EPYC with no `/dev/kvm`; an older forum thread says nested virtualisation is disabled. I found no official Hetzner statement. Test `ls -l /dev/kvm` on the actual instance type [S: Itegro-GmbH GitHub issue, k7 README, pandastack roundup].
- Hetzner dedicated: real `/dev/kvm`, occasionally disabled in firmware at delivery.

What existing platforms use [S: northflank blog, dev.to/zeropsio comparison, pethost, fly.io]:

| Platform | Isolation |
|---|---|
| Fly.io | Firecracker microVMs |
| Northflank | Kata Containers on Cloud Hypervisor, plus Firecracker and gVisor where needed; gVisor fallback where nested virtualisation is unavailable |
| Koyeb | Kata Containers on Cloud Hypervisor (moved away from Firecracker); documentation describes instances as microVMs |
| Render | Disputed: one source says shared-kernel containers; one says Firecracker; the second may refer to a sandbox product |
| Railway | Containers (documentation names no sandbox runtime) |
| Modal | gVisor (from general knowledge, not verified this session): UNVERIFIED |
| Shuttle | Hosted service is shutting down (Pro projects stopped 2026-01-16); no isolation data |

Recommendation (cheapest safe):
1. **Launch:** gVisor RuntimeClass on every tenant pod, on bare metal with the KVM platform or on cheap VMs with systrap, plus NetworkPolicy default-deny, Pod Security `restricted`, `automountServiceAccountToken: false`, read-only rootfs, no host networking, egress filtering for SMTP and mining pools, CPU and memory limits, per-tenant namespaces. Cost: ≈ 50 MB RAM per pod, no extra hardware.
2. **Growth:** move to bare metal (AX102/AX162) and offer Kata + Cloud Hypervisor (or Firecracker) as the default for paid tenants. Keep gVisor for free tier and for burst nodes on Hetzner Cloud VMs.
3. Do not use plain runc with namespaces alone for third-party code.

### Q4. Scale to zero and autoscaling

| Option | Mechanism | Cold start for a small Rust binary | Always-on overhead | Source |
|---|---|---|---|---|
| Knative Serving | Activator buffers first request; queue-proxy sidecar on every pod; own CRDs and networking layer | ≈ 3.2 s for a trivial image on kind; 30–60 s with large images; dominated by scheduling + pull + readiness | Heavy control plane (controller, activator tier, networking layer) | [S] thinhdanggroup, bex.co 2026-09 |
| KEDA + HTTP add-on | KEDA does 0→1 and 1→0, HPA does 1→N; interceptor proxy holds the first request | Same order, a few seconds | Operator + metrics server + interceptor (a few hundred millicores, never scales to zero); HTTP add-on was beta (check current status) | [S] |
| Sablier (proxy-layer waker for Traefik/Caddy) | Wakes a deployment from the proxy | Same order | Very small | [S] bex.co |
| **Own waker in the platform's ingress (recommended)** | Your own Rust router holds requests and scales a Deployment 0→1 | Pod scheduling + sandbox create + process start; a static Rust binary starts in ms; expect ~0.5–1.5 s under gVisor with cached image, 1–5 s with Kata (measured 4.76 s for Kata QEMU on a nested host) | One small router | calc and [S] computingforgeeks benchmark |
| Fly Machines autostop/autostart | Fly Proxy stops/suspends on low load, starts on request | Cold start ≈ 2 s+ for common apps; **suspend/resume a few hundred ms**; third-party tests 1.8–4.2 s for Node | Stopped machine rootfs $0.15/GB-month | [S] docs.fly.io suspend-resume page; jakeinsight benchmark |
| Cloud Run | Request-based billing, min instances 0 | Seconds; per-request billing | none | UNVERIFIED rates |
| Cloudflare Containers | Sleeps when idle, billed only while active | Not benchmarked here: UNVERIFIED | none | [P] |

Idle cost per sleeping app: effectively storage only: image (30–150 MB) ≈ $0.001–0.002/month at R2 rates plus a small database ≈ $0.01–0.02 (calc). A stopped Fly machine costs $0.15/GB of rootfs, so ~$0.02 for a 128 MB image [P].

Density: how many 64 MB and 128 MB apps fit on one node (calc; reserve 1 GB / 1.5 GB / 2 GB for OS, kubelet, agents, Postgres cache on 4/8/16 GB nodes; sandbox overhead gVisor 50 MB, Kata 130 MB). These are *active* apps. Sleeping apps cost zero RAM, so registered capacity is multiples of this.

| Node | Usable | runc 64 MB | runc 128 MB | gVisor 64 MB | gVisor 128 MB | Kata 64 MB | Kata 128 MB |
|---|---|---|---|---|---|---|---|
| 4 GB | 3.0 GB | 47 | 23 | 26 | 17 | 15 | 11 |
| 8 GB | 6.5 GB | 101 | 50 | 57 | 36 | 33 | 25 |
| 16 GB | 14 GB | 218 | 109 | 122 | 78 | 72 | 54 |
| 64 GB (AX41) | 56 GB | 875 | 437 | 491 | 314 | 289 | 217 |
| 128 GB (AX102) | 116 GB | 1,812 | 906 | 1,017 | 651 | 598 | 449 |

If 10% of apps are awake at any time, multiply by about 10 for registered apps per node (memory only; also check CPU, IO and the Postgres share).

### Q5. Postgres

| Option | Price | Storage | Backups / PITR | Notes | Source |
|---|---|---|---|---|---|
| **CloudNativePG on own nodes** (recommended) | node share; ~€0.02/GB-month NVMe on AX | local NVMe | WAL + base backups to R2 ($0.015/GB-month) or B2 ($6.95/TB-month) or Hetzner Object Storage (€6.49 base incl. 1 TB) | One cluster per tier; database-per-tenant in a shared cluster for hobby, dedicated cluster for paid | calc; prices [P] |
| Shared Postgres, database per tenant | ≈ €0.02–0.30 per small DB (calc) | | same | Needs per-DB connection limits, pgbouncer, role isolation, noisy-neighbour controls | calc |
| Neon (hosted) | Free: 100 CU-hours/project, 1 GB/project, scale to zero after 5 min. Launch: $0.106/CU-hour, $0.35/GB-month, PITR up to 7 days (instant restore $0.20/GB-month). Scale: $0.222/CU-hour | $0.35/GB-month | | Interesting as upstream; per-tenant billing too high for margin | [P] neon.com/pricing |
| Self-hosting Neon | open source, but needs pageserver, safekeepers and storage controller | | | Operationally heavy: UNVERIFIED this session, general knowledge | |
| Supabase | Free 500 MB; Pro from $25 including $10 compute credit; Micro $10 (1 GB RAM), Small $15 (2 GB); 8 GB DB included then $0.125/GB; PITR $100/month per 7 days | | | | [P] supabase.com/pricing |
| Crunchy Bridge | Hobby-2 (1 core/2 GB) **$35/month**; Standard-4 $70; storage **$0.10/GB-month**; daily + WAL backups and PITR included | | included | | [P] crunchydata.com/pricing |
| DigitalOcean managed | 1 vCPU / 1 GiB from **$15.15/month**; storage $0.215/GiB-month (10–30 GiB range on cheapest plan); standby price not on page | | backups not on page | | [P] |
| PlanetScale Postgres | PS-5 (1/16 vCPU, 512 MiB) **$15/month as 3-node HA**, $5 non-HA; storage and backups billed separately (prices not on page) | | | | [P] planetscale.com/pricing |
| Xata Cloud | micro from **$9/month** ($0.012/hour), storage **$0.28/GB-month**; branching copy-on-write; scale to zero (~1 s wake); open-source self-host option (~10 s wake) | | | | [P] xata.io/pricing |
| Ubicloud managed Postgres | hobby-1 (1 shared vCPU, 2 GB, 16 GB NVMe) **$19.54/month**, standard-2 (2 vCPU, 8 GB, 64 GB) $78.15; US +20%; per-minute billing | extra storage not priced on page | daily backups + 7-day PITR included | Runs on Hetzner Germany/Finland | [P] ubicloud.com |
| Fly Managed Postgres | $38/month Basic; storage $0.28/GB-month | | | | [P] |
| Koyeb Serverless Postgres | Free 5 h; Small $0.04/h (≈$29.76/month); Medium $0.08/h; storage $0.50/GB-month | | | | [P] koyeb.com/pricing |
| Render Postgres | Basic-256mb **$6**, Basic-1gb **$19**, Basic-4gb $75 | | | | [S] |
| Heroku Postgres | Essential-0 $5 (1 GB, 20 connections), Essential-1 $9 (10 GB), Essential-2 $20, Standard-0 $50, Premium-0 $200 | | | | [P] heroku.com/pricing |
| Civo managed | Small (4 GB, 2 cores, 40 GB) $43.45/month; Medium $86.91 | | | | [P] |
| Serverpod Cloud DB | S (1 GB) $0.043/h ≈ $31/month; storage + backup storage $0.56/GB-month | | | | [P] |
| Scaleway serverless SQL, Aiven | not fetched | | | UNVERIFIED | |
| Hetzner managed DB | none found in any source | | | | [S] |

Backup storage candidates [P]: R2 $0.015/GB-month, no egress; B2 $6.95/TB-month, free egress to 3× average storage, Class A/B/C free; Hetzner Object Storage €6.49 for 1 TB + 1 TB egress, extra €0.0087 per TB-hour (≈ €6.35/TB-month), requests free; Wasabi $7.99/TB, 1 TB minimum, 90-day minimum retention, no egress or API fees.

### Q6. Object storage, CDN, DNS, TLS, registry

#### Object storage

| Provider | Storage | Operations | Egress | Source |
|---|---|---|---|---|
| Cloudflare R2 | $0.015/GB-month Standard; $0.010 Infrequent Access (30-day minimum, $0.01/GB retrieval) | Class A $4.50/M; Class B $0.36/M; deletes free | **free** (Workers API, S3 API, r2.dev) | [P] developers.cloudflare.com/r2/pricing |
| R2 free tier | 10 GB-month, 1M Class A, 10M Class B | | | [P] |
| Backblaze B2 | $6.95/TB-month ($0.00695/GB), first 10 GB free | Class A/B/C free; Class D $0.004 per 10,000 (first 2,500/day free) | free up to 3× average monthly storage, then $0.01/GB; free via partner CDNs (including Cloudflare) | [P] backblaze.com/cloud-storage/pricing |
| Hetzner Object Storage | €6.49/month includes 1 TB storage + 1 TB egress; extra €0.0087/TB-hour | S3 requests free; objects <64 KB billed as 64 KB | extra €1.00/TB; ingress and eu-central internal traffic free | [P] hetzner.com/storage/object-storage; locations FSN1, HEL1, NBG1 |
| Wasabi | $7.99/TB-month, 1 TB minimum, 90-day minimum | none | none | [P] wasabi.com/pricing |
| Bunny Storage | Standard HDD $0.01/GB (1 region), $0.02 (2 regions); Edge SSD $0.02/GB/region; $1 monthly minimum | no API fees | free to Bunny CDN | [P] bunny.net/pricing/storage |
| Railway object storage | $0.015/GB-month, free egress | | | [P] |
| Scaleway, OVH object storage | not fetched | | | UNVERIFIED |

#### CDN and edge

| Product | Price | Note | Source |
|---|---|---|---|
| Cloudflare Free plan | $0 | Do not give customers accounts (§2.2.1(a)); CDN non-HTML limits in service terms not verified | [P] terms |
| Cloudflare for SaaS custom hostnames | **100 hostnames free; then $0.10 per hostname per month**; cap 50,000 on Free/Pro/Business, Enterprise beyond | Fee is on top of base plan. 1,000 hostnames ≈ $90/month, 10,000 ≈ $990 [S: Cloudflare blog "waf-for-saas", domainee.dev, Cloudflare Community] | [S]; the Cloudflare docs page confirms 100 included, 50,000 max, $0.10 [P] |
| Bunny CDN | $0.01/GB EU/NA, $0.03 Asia/Oceania, $0.045 South America, $0.06 Middle East/Africa; Volume network $0.005/GB; $1 monthly minimum; no request fees | | [P] bunny.net/pricing |
| Fastly | not fetched | | UNVERIFIED |
| Hetzner | unlimited traffic on dedicated; €1/TB beyond 20 TB on cloud | | [P] |

#### Wildcard TLS and customer domains

| Approach | Cost | Trade-off |
|---|---|---|
| Wildcard `*.platform.dev` via Let's Encrypt DNS-01 (cert-manager) | $0 | Needs DNS provider API (Cloudflare DNS free) |
| Customer custom domains with **Cloudflare for SaaS** | first 100 free then $0.10/hostname/month | Offloads cert issuance, DDoS and CDN; vendor dependency |
| Customer custom domains with **Caddy on-demand TLS or cert-manager HTTP-01** | $0 | You run it; Let's Encrypt rate limits apply per registered domain (general knowledge; UNVERIFIED this session); needs a stable public IPv4 per ingress and you need abuse/ownership checks before issuing |

Recommendation: wildcard on the platform domain through cert-manager; custom domains through Cloudflare for SaaS up to a few thousand hostnames (≈ $0.10 each is cheaper than operating issuance and DDoS protection yourself), and revisit Caddy on-demand TLS if hostnames exceed ~10,000.

#### Private OCI registry

| Option | Cost | Note | Source |
|---|---|---|---|
| GHCR | Free today; sources disagree on private image pricing ($0.25/GB-month storage and $0.50/GB transfer in one) | GitHub says it will give notice before charging; not a contractual guarantee | [S] |
| Self-hosted Zot or Distribution with S3 backend on R2 | R2 $0.015/GB-month, free egress; Hetzner Object Storage €6.49 for 1 TB | You own uptime | [S] |
| Cloudflare serverless-registry (Workers + R2) | R2 + Workers request charges | Experimental, 2024 project | [S] |
| Docker Hub | Pro $9/user/month annual; Team $15; personal free but 1 private repo | Per seat, poor fit | [S] |
| ECR / Google Artifact Registry | ≈ $0.10/GB-month plus egress | | [S] |
| Fly registry/builders | currently free, may change | | [P] |

Recommendation: zot on a node (with R2 backend) for tenant images, deduplicated by layer, plus GHCR for your own platform images while it stays free.

### Q7. Build capacity

#### Linux CI per-minute prices

| Service | Price | Note | Source |
|---|---|---|---|
| GitHub Actions standard Linux x64 2-core | **$0.006/min** | arm64 2-core $0.005; slim 1-core $0.002; larger Linux $0.006 (2-core) to $0.252 (96-core); partial minutes rounded up; larger runners only on Team/Enterprise | [P] docs.github.com actions runner pricing |
| Ubicloud runners | **$0.00125/min** standard x64 2 vCPU/8 GB; $0.002 premium; arm64 $0.00125; $2.50/month credit = 1,250 min | Runs on Hetzner; up to 30 vCPUs | [P] ubicloud.com/docs/about/pricing |
| Blacksmith | **$0.004/min** Ubuntu x64; $0.0025 ARM; macOS M4 $0.08; 3,000 free minutes/month; Docker layer cache +$0.50/GB-month | | [P] blacksmith.sh/pricing |
| WarpBuild | $0.004/min (2 vCPU) up to $0.064 (32 vCPU); arm64 $0.003; macOS M4/M5 Pro $0.08 (6 vCPU) or $0.16 (12 vCPU); **BYOC $0.002/min**; cache storage $0.20/GB-month | | [P] warpbuild.com/pricing |
| Namespace | Unit minute = 1 vCPU/2 GB; $0.001/min prepaid, $0.0015 overage; multipliers Linux ×1, Linux on Apple silicon ×7, macOS ×10; Team $100/month incl. 100,000 unit minutes | | [P] namespace.so/pricing |
| Cirrus Runners | **$150/runner/month, unlimited minutes**: macOS (4 M4 Pro cores, 16 GB), Linux x86 (16 vCPU, 48 GB, KVM), Linux arm64 (8 vCPU, 24 GB) | | [P] cirrus-runners.app/pricing |
| Depot, BuildJet | not fetched | | UNVERIFIED |
| Hetzner ephemeral VMs (hetzner-k3s, ARC, custom) | Hourly billing with monthly cap; e.g. CPX42 8 vCPU €0.0953/h ≈ €0.0016/min | Per-hour billing makes short jobs expensive unless a warm pool is reused | [P] |
| **Own build box (Hetzner AX42, €79)** | ≈ €0.0009 per 4-thread minute at 50% utilisation (section 2) | Add sccache + registry cache on NVMe; no per-job boot | calc |

Rust note: CPU-heavy builds finish faster on bigger runners, so compare on cost per build, not per minute. A fast 16-thread box may cost the same per build as a 2-vCPU runner and finish in a quarter of the time.

#### macOS and iOS

| Option | Price | Minimum | Source |
|---|---|---|---|
| **Owned Mac minis** (you have three) | power + connection | none | |
| GitHub Actions macOS standard (3–4 core) | **$0.062/min**; 12-core $0.077; M2 Pro 5-core arm64 $0.102 | | [P] |
| Blacksmith macOS M4 | $0.08/min | | [P] |
| WarpBuild macOS M4/M5 Pro | $0.08/min (6 vCPU), $0.16 (12 vCPU) | | [P] |
| Codemagic | pay-as-you-go Mac mini M2 $0.095/min, **M4 $0.114/min**, Linux/Windows $0.045; free 500 M2 min/month; fixed plans $3,990/yr (M2), $5,400/yr (M4), $9,000/yr (M4 Max) unlimited with 3 concurrent builds | | [P] codemagic.io/pricing |
| Bitrise | Hobby free 500 min; Starter $89/month (annual); Pro $200/month; M4 Large listed at $0.0096/min and Build Hub M4 Large $0.03/min (PAYG $0.06): credit-based, treat per-minute figures as UNVERIFIED | | [P] bitrise.io/pricing |
| Expo EAS Build | Free: 15 Android + 15 iOS builds; Starter $19/month (+$45 credit); Production $199/month (+$225 credit); build rates Android $1 medium / $2 large, **iOS $2 medium / $4 large** | | [P] expo.dev/pricing |
| Cirrus Runners macOS | $150 per runner per month, unlimited minutes | | [P] |
| Scaleway Apple silicon (Paris) | Mac mini M4-S (16 GB/256 GB) **€0.22/h or €149/month**; M4-SP €0.24/h or €165; M4-M €0.29/h or €199; M4 Pro €0.49/h or €335; **24 h minimum** | 24 h hourly; 1 month monthly | [S] scaleway.com pages via search; official page 403 |
| MacStadium | M4 (16 GB/256 GB) **$149/month**, M4 24 GB/512 GB $249, M4 Pro 48 GB $349; M2 8 GB $109 (secondary); colocation and Orka pricing not listed | monthly | [P] macstadium.com/pricing |
| AWS EC2 Mac | mac-m4 $1.23/h (us-east-1; $1.353 Ireland, $1.476 Frankfurt) ≈ $898/month; mac-m4pro $1.97/h; mac2 M1 $0.650/h; **24 h minimum** ($29.52 for the smallest M4 allocation); Savings Plans up to 44% off with 3-year commitment | 24 h | [P] AWS page confirms 24 h minimum and 44% max discount; hourly figures [S] macrun.dev, macyou.co |
| MacinCloud | Pay-as-you-go from $4/day; Managed Server from $25/month; Dedicated from $49/month | | [S] macswarm.com 2026-10-26 |
| Hetzner | **No Mac mini offered** (dedicated range lists none) | | [S] macswarm.com, [P] AX matrix |
| OVH | no Mac offer found | | UNVERIFIED |
| Apple Mac mini M4 list price | $599 for 16 GB/256 GB | | UNVERIFIED (from memory) |

Cost per macOS minute (calc, 43,200 min/month):

| Basis | 100% utilisation | 40% | 10% |
|---|---|---|---|
| Scaleway M4-S €149/month | €0.0034 | €0.0086 | €0.034 |
| MacStadium M4 $149/month | $0.0034 | $0.0086 | $0.034 |
| AWS mac-m4 $898/month | $0.0208 | $0.052 | $0.208 |
| Owned mini fully amortised (~$20/month) | $0.0005 | $0.0012 | $0.0046 |
| GitHub-hosted | $0.062 flat | | |

Rented minis only beat GitHub's $0.062 per minute if utilisation stays above roughly 6% (MacStadium/Scaleway). Owned minis break even almost immediately.

#### Apple licence restrictions (section 3 of the macOS Software License Agreement, Sequoia text) [S]

- Leasing a Mac requires each lease period to be **at least 24 consecutive hours**, and the lessee must have sole and exclusive use and control of the leased machine. This is why AWS and Scaleway have a 24-hour minimum.
- Section 2B(iii) allows up to **two additional virtual copies** of macOS on each Apple-branded Mac you own or control, for software development, testing during development, macOS Server, or personal non-commercial use. It does **not** permit use of virtual copies for "service bureau, time-sharing, terminal sharing or similar services" except as permitted under Section 3 (leasing).
- Implication: offering customers per-minute builds on shared Macs is a legal grey area that CI vendors handle with their own arrangements. Before selling iOS build minutes, take legal advice and either (a) lease whole Macs to a customer for ≥24 h with exclusive use, or (b) build only your own first-party apps. Do not rely on this summary; read the current macOS SLA.

### Q8. Competitor reference pricing

Dates are the read date (2026-11-18) unless noted. [P] = provider page, [S] = secondary.

| Competitor | Free tier | Entry paid | Usage rates | Source |
|---|---|---|---|---|
| **Serverpod Cloud** | Starter $5/month with $5 usage credit and trial button; no free tier stated | Growth $20/month + usage; Enterprise $799/month | Podlets S 256 MB $0.026/h (≈$19/month), M 512 MB $0.040/h (≈$29), L 1 GB $0.081/h (≈$58); DB S 1 GB $0.043/h (≈$31); DB storage and backup $0.56/GB-month, DB network $0.16/GB; file storage $0.024/GB-month, egress $0.15/GB; extra domains $9 each | [P] serverpod.dev/cloud |
| **Fly.io** | none for new orgs ($5 bill waived for personal orgs; $1 trial credit) | pay-as-you-go | See Q1 table: from $2.19 (shared-cpu-1x 256 MB); egress $0.02/GB; MPG from $38 | [P] |
| **Railway** | Free $1/month credit; Trial $5 one-time | Hobby $5 (includes $5 usage); Pro $20 per workspace (includes $20) | $20/vCPU-month ($0.00000772/vCPU-s), $10/GB-month, volumes $0.15/GB-month, egress $0.05/GB, object storage $0.015/GB-month | [P] railway.com/pricing |
| **Render** | Hobby $0 | Starter instance $7 (512 MB, 0.5 CPU); Standard $25 (2 GB, 1 CPU); Pro $85 (4 GB, 2 CPU); **Pro workspace flat $25** (since Aug 2026 rollout of the 23 Apr replan); Scale $499 | Postgres Basic-256mb $6, 1gb $19, 4gb $75; bandwidth included 5 GB Hobby / 25 GB Pro / 1 TB Scale then $0.15/GB | [S] (Render pricing page did not render) |
| **Vercel** | Hobby $0 (100 GB bandwidth, 1M invocations) | Pro $20 including $20 credit | Fast Data Transfer $0.15/GB over 1 TB, Active CPU from $0.128/CPU-hour, provisioned memory from $0.0106/GB-hour, invocations $0.60/M | [P] |
| **Netlify** | Free 300 credits/month, hard cap | Personal $9 (1,000 credits); Pro $20 flat, unlimited members (since 2026-04-14), 3,000 credits; Pro credit tiers 5,000 credits $33/month to 20,000 credits $126/month | production deploy 15 credits; bandwidth 20 credits/GB; compute 10 credits/GB-hour | [S] dev.to, stork.ai |
| **Heroku** | none | Eco $5 (sleeps 30 min), Basic $7, Standard-1X $25, Performance-M $250, Performance-L $500 | Postgres Essential-0 $5, Essential-1 $9, Standard-0 $50, Premium-0 $200 | [P] heroku.com/pricing |
| **Northflank** | Sandbox: up to $20/month of resources, 2 services, 2 jobs, 1 database | Pay as you go | $0.01667/vCPU-hour (≈$12.2/vCPU-month), $0.00833/GB-hour (≈$6.1/GB-month), disk $0.11657/GiB-month, egress $0.06/GB; plans 0.5 vCPU/1 GB $12, 1/2 $24; BYOC $0.02083/vCPU-hour | [P] northflank.com/pricing |
| **Koyeb** | Free tier existed per aggregator; not shown on page | Pro $29/month incl. $10 compute (1 TB bandwidth); Scale $299 incl. $100 | Bandwidth beyond 1 TB $0.02/GB EU/US, $0.04 Asia; Postgres from $0.04/h; storage $0.50/GB-month; ten custom domains free then $0.20 each | [P] koyeb.com/pricing |
| **Shuttle** | hosted service shutting down (Pro projects stopped 2026-01-16) | none to buy | historical $20/month + usage | [S] sandbox0.ai |
| **Encore Cloud** | Free "Starter": 2 dev environments, 1M tracing events | **Pro $49 per team member/month + $99 per AWS/GCP environment + $2.50 per resource per environment** | cloud usage billed by AWS/GCP; Pro calculator example 3 members, 1 environment, 10 resources = $271/month before cloud | [P] encore.dev/pricing |
| **Leapcell** | Hobby $0/seat; 20 projects up to 3 vCPU/4 GB; 1 free Postgres; 1M invocations | listed from $12.90/month [S] | per-ms compute, $0.60/M invocations over 1M | [S] agentdeals.dev, aggregators (free terms unverified on 2026-10-28) |
| **Zeabur** | Free trial $5 monthly credit, ≤1 vCPU/2 GB per service | Developer $5/month; Team $80 (plans differ across sources) | memory $0.00025/GB-minute (≈$10.80/GB-month), CPU free, egress $0.10/GB, volumes $0.20/GB | [S] getdeploying, toolradar |
| **Sevalla** (ex Kinsta) | static sites: 100 sites, 100 GB bandwidth | usage-based | 1 vCPU/1 GB VM $18/month; 4 vCPU/8 GB $160; egress $0.10/GB over allowance | [S] getdeploying, wheretodeploy |
| **Coolify Cloud** | none (software is open source) | $5/month for 2 servers, +$3 per extra server; 20% off annual | you pay your own servers | [S] pethost.dev, dev.to |
| **Dokploy Cloud** | | Hobby $4.50 per server per month; Startup $15 for 3 servers, +$4.50 each | you pay your own servers | [S] |
| **Expo EAS** | Free: 15 Android + 15 iOS builds/month, 1,000 update MAUs | Starter $19 + usage ($45 credit, 3,000 MAUs); Production $199 + usage ($225 credit, 50,000 MAUs) | Android build $1 / $2, iOS $2 / $4 (medium / large); Updates $0.005/MAU (3,001–200,000 on Starter, 50,001–200,000 on Production), lower above | [P] expo.dev/pricing |
| **Codemagic** | 500 M2 minutes/month for individuals | Pay-as-you-go | M2 $0.095/min, M4 $0.114/min, Linux/Windows $0.045/min; fixed $3,990–9,000/year; Patch OTA: first 100,000 MAU free, then $99 per extra 100,000 MAU | [P] codemagic.io/pricing |

Reference workload from a Sept 2026 comparison [S: bex.co 2026-09-19] (1 web service ~1 vCPU/2 GB, 1 worker, ~10 GB Postgres, 3 previews, 5 seats, 100 GB egress): Render ≈ $74, Heroku ≈ $69, Fly ≈ $72, Railway ≈ $122, Hetzner dedicated ≈ $45–55 self-run.

---

## 4. Risks

1. **Hetzner price and stock volatility.** Two 2026 price rounds; cheap CX/CAX showed "Currently unavailable" on the official page; per-region differences (HEL cheaper than NBG; US and Singapore have far smaller traffic allowances: 0.5–5 TB vs 20 TB). Mitigation: keep the platform provider-neutral (Cluster API/Talos or k3s), keep a second provider (OVH, Netcup) pre-tested.
2. **Terms about reselling and untrusted workloads.** Hetzner explicitly allows granting third parties use but holds you liable for their content; crypto mining is prohibited, so enforce it in the sandbox. Cloudflare §2.2.1(a) prohibits selling access to Cloudflare services or signing customers up on their behalf; use Cloudflare for SaaS, not shared accounts. Read the AUPs for any other provider before signing.
3. **Abuse on free tiers.** Crypto mining, spam relays, DDoS, phishing, proxies. Controls: no outbound SMTP by default, egress allow-lists or rate limits, CPU throttling and hard caps, card or phone verification before always-on, automated mining-pool domain and signature blocking, abuse inbox with 24 h response, per-tenant kill switch.
4. **Noisy neighbours and kernel escapes.** gVisor reduces but does not eliminate kernel exposure; Kata/Firecracker give a stronger boundary but need KVM (bare metal). CPU oversubscription at 3:1 needs per-pod limits and `cpu.max` throttling. IO and Postgres noisy neighbours need connection limits, statement timeouts and per-database resource caps.
5. **IPv4 costs.** Hetzner Cloud IPv4 €0.50/month each (billed even if unattached); Ubicloud $3/month; Fly dedicated IPv4 $2/month; DO's v5 IPv4 is $0.00 introductory and "subject to change". Use shared ingress IPs, SNI, and Cloudflare in front; IPv6-only for backends.
6. **Egress surprises.** Hetzner cloud overage €1/TB EU/US but **€7.40/TB Singapore** with only 0.5 TB included on the smallest plans; hyperscalers and Railway ($0.05/GB), Render ($0.15/GB), Vercel ($0.15/GB) bill much more; B2 charges $0.01/GB above 3× storage; Cloudflare Containers $0.025/GB after 1 TB. Hetzner dedicated "unlimited" is a fair-use policy, not a contract; the policy excludes servers with the 10G uplink add-on.
7. **Single points of failure at launch.** One AX41 is a single failure domain. Backups to R2/B2 are mandatory; test restores.
8. **Postgres multi-tenancy.** A shared Postgres cluster with a database per tenant needs role isolation, per-DB connection caps, extension allow-lists and a clear upgrade path to dedicated clusters.
9. **Apple licence.** See Q7 above; get legal advice before selling per-minute macOS builds.
10. **Data quality of this research.** Several providers' official pricing pages were blocked or incomplete (Scaleway, Vultr, Render, Cloud Run, Azure, Linode, Bitrise credits). Everything marked [S] or UNVERIFIED should be re-checked before it appears in a price list or contract.

---

## 5. Source list (read 2026-11-18 unless noted)

Provider pages [P]:
- https://www.hetzner.com/cloud/ , /cloud/regular-performance/ , /cloud/general-purpose/ , /cloud/cost-optimized/
- https://www.hetzner.com/dedicated-rootserver/matrix-ax/ , https://www.hetzner.com/sb/
- https://www.hetzner.com/storage/object-storage/ , https://www.hetzner.com/terms-and-conditions/
- https://www.ovhcloud.com/en/vps/ , https://www.ovhcloud.com/en/public-cloud/kubernetes/
- https://www.netcup.com/en/server/vps , https://contabo.com/en/vps/
- https://www.digitalocean.com/pricing/droplets , /pricing/kubernetes , /pricing/managed-databases
- https://www.akamai.com/cloud/pricing
- https://docs.fly.io/about/pricing
- https://aws.amazon.com/fargate/pricing/ , https://aws.amazon.com/ec2/instance-types/mac/
- https://azure.microsoft.com/en-us/pricing/details/container-apps/
- https://developers.cloudflare.com/containers/pricing/ , /r2/pricing/ , /workers/platform/pricing/ , /cloudflare-for-platforms/cloudflare-for-saas/plans/ , https://www.cloudflare.com/terms/
- https://www.backblaze.com/cloud-storage/pricing , https://www.wasabi.com/pricing , https://bunny.net/pricing/ , https://bunny.net/pricing/storage/
- https://www.civo.com/pricing
- https://neon.com/pricing , https://supabase.com/pricing , https://www.crunchydata.com/pricing , https://planetscale.com/pricing , https://xata.io/pricing , https://www.ubicloud.com/use-cases/postgresql , https://www.ubicloud.com/docs/about/pricing
- https://docs.github.com/en/billing/reference/actions-runner-pricing
- https://www.blacksmith.sh/pricing , https://namespace.so/pricing , https://www.warpbuild.com/pricing , https://cirrus-runners.app/pricing/
- https://www.macstadium.com/pricing
- https://codemagic.io/pricing/ , https://bitrise.io/pricing , https://expo.dev/pricing
- https://railway.com/pricing , https://www.koyeb.com/pricing , https://northflank.com/pricing , https://www.serverpod.dev/cloud , https://vercel.com/pricing , https://www.heroku.com/pricing/ , https://encore.dev/pricing

Secondary sources [S]:
- Hetzner 2026 price changes: https://trackvps.com/hetzner-cloud-pricing , https://trackvps.com/hetzner-price-increase , https://costgoat.com/pricing/hetzner (updated 2026-10-20), https://bex.co/blog/2026/10/14/hetzner-2026-price-hikes-traffic-trap , https://northflank.com/blog/hetzner-cloud-server-price-increases , https://betterstack.com/community/guides/web-servers/hetzner-cloud-review/
- Hetzner Kubernetes: https://ankra.ai/blog/kubernetes-on-hetzner-options-2026 , https://www.qovery.com/blog/eu-cloud-providers-managed-kubernetes-2026-sovereignty-checklist
- Scaleway Kapsule and Mac: search results quoting scaleway.com/en/pricing, https://www.scaleway.com/en/pricing/apple-silicon/ , https://stackrates.ai/hosting/scaleway/ , https://calculator.holori.com/scaleway
- Mac rental comparison: https://macswarm.com/compare/cloud-mac-pricing/ , https://macrun.dev/ec2-mac-alternative , https://macyou.co/blog/aws-ec2-mac-pricing-explained
- Apple licence: https://www.apple.com/legal/sla/docs/macOSSequoia.pdf (via search excerpts), https://threedots.ovh/blog/2020/12/macos-eula-licensing-restrictions-affecting-virtualisation/
- Isolation: https://gvisor.dev/docs/architecture_guide/platforms/ , https://computingforgeeks.com/gvisor-vs-kata-containers-vs-runc/ , https://bex.co/blog/2026/07/09/kata-containers-vs-gvisor-runtimeclass-selection , https://northflank.com/blog/kata-containers-vs-firecracker-vs-gvisor , https://pethost.dev/blog/paas-on-your-own-vm/ , https://github.com/Itegro-GmbH/code-reviewer/issues/1005 , https://www.pandastack.ai/blog/best-bare-metal-providers-for-firecracker-2026/
- Scale to zero: https://bex.co/blog/2026/09/07/keda-http-addon-vs-knative-vs-sablier-scale-to-zero , https://www.fly.io/docs/reference/suspend-resume/ , https://jakeinsight.com/productivity/2026-03-29-flyio-free-tier-cold-start-latency-nodejs-api-real/
- Competitors: https://bex.co/blog/2026/09/19/paas-pricing-shakeup-ledger-render-heroku-hetzner , https://sandbox0.ai/alternatives/shuttle , https://getdeploying.com/sevalla-vs-zeabur , https://agentdeals.dev/vendor/leapcell , https://pethost.dev/blog/coolify-cloud-vs-dokploy-cloud/ , https://dev.to/nayankyada/netlify-pricing-2026-free-tier-limits-build-minutes-when-to-upgrade-476f
- Cloudflare for SaaS: https://domainee.dev/blog/cloudflare-for-saas-pricing , https://blog.cloudflare.com/waf-for-saas/
- Registries: https://hippius.com/blog/container-registry-pricing , https://ohmyops.dev/tools/container-registry-small-teams/
- Netcup: https://pricechangenews.com/netcup-update-on-hosting-price-increase-due-to-memory-shortage/ , https://www.netcup.com/en/server/root-server/rs-1000-g12-ip-iv-12m
- OVH/Kimsufi: https://stackrates.ai/hosting/kimsufi/ , https://blog.ovhcloud.com/en/posts/dedicated-servers-pricing-update-h2-2026/
