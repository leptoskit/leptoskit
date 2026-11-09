# Graph edge routing

The deployment graph generates one Caddy JSON configuration per host with public routes. Private-only hosts receive none. The current acceptance baseline is the official Caddy 2.11.9 binary. Generation records that version and leaves installed-binary and running-route evidence `not_checked`.

## Routing contract

The edge selects the longest case-sensitive mount on the declared hostname, using segment boundaries. A bare mount redirects with its query string preserved. Static mounts strip their prefix before looking in that unit's captured asset tree; SSR and service proxies preserve the complete application path and query. An SSR application mounted at `/play/` therefore must serve `/play/` itself.

Static requests support GET, HEAD and ranges, with `index.html` as the only index candidate. Missing assets return 404 within the selected owner. Directory browsing and SPA fallback are not enabled. Unknown hosts receive 404. Host matching accepts DNS case changes, a terminal dot and the parsed Host port. HTTPS additionally requires Host and TLS SNI to agree, so a trailing dot must be consistent at that boundary. Encoded separators, dot segments, repeated separators and ASCII control characters are refused before routing. Mount declarations containing placeholder braces are refused rather than interpreted by Caddy.

The static root is `/var/lib/leptoskit/<project>/units/<qualified-unit>/current/assets`. It must contain the checked public tree, with no private application files or symlinks in that tree. Local capture already checks this; publication and ongoing host integrity still require the graph release lifecycle. This edge configuration is not a filesystem sandbox against a compromised origin host.

## Ingress and application identity

HTTPS listens on 443, loads explicit per-hostname origin certificate files from `/etc/leptoskit/<project>/tls`, disables automatic certificate acquisition, and trusts only the checked Cloudflare address snapshot. Tunnel listens on `127.0.0.1:8088`, trusts loopback, and expects the managed local connector to supply `CF-Connecting-IP`. Processes already on that origin host are part of this trust boundary. TLS/Cloudflare configuration and connector installation have not been automated by this slice.

Proxies remove supplied forwarding headers before installing the edge's canonical Host, forwarded host, HTTPS scheme and resolved client IP. Upstreams bind to loopback; conflicting edge listener ports are rejected. Graph services strip request Cookie and response Set-Cookie by default. Only an explicit `trust = "first-party"` retains those ambient credentials; package location does not grant trust. Authorization remains available for explicit service credentials. SSR frontends retain their own sessions.

The `calls` relationship is not authentication, authorization or a network grant. CORS, CSP and workload isolation require their own contracts. Header filtering does not make the untrusted compilation service safe to run; its exclusive host, restricted runtime, limits and pinned toolchain still need acceptance.

## Local acceptance

The ignored test `infra::graph::edge::tests::generated_edges_serve_only_owned_routes_and_verified_proxy_headers` takes `LEPTOSKIT_TEST_CADDY`, validates its version, renders the production configurations, and executes `tests/fixtures/graph-edge-acceptance.py`. Only deployment bindings change: temporary public roots, listener/upstream ports and ephemeral certificates. Routing and header rules remain the generated production output.

The fixture uses real local HTTP, a temporary certificate trusted explicitly by the TLS client, and an actual HTTP backend. It exercises static ownership, case/host handling, redirects, query strings, nested assets, missing assets, Wasm MIME, HEAD/ranges, methods, proxy identity, cookie isolation, SSR ownership and TLS Host/SNI rejection. A required negative control removes docs ownership and confirms the root sentinel becomes visible. Its processes and temporary private key are removed afterward.

Set `LEPTOSKIT_EDGE_EVIDENCE` to retain public configurations, logs, the public test certificate and hashes on either success or failure. No private key is retained. The parent run compiled the production configuration, network, provider, report, generation and edge code against cached dependencies. It passed 19 production unit tests and the real ignored journey, with workspace Clippy. A twentieth passing harness test merely references the reporting entry point; it is not application acceptance. The harness omits the unrelated build module and substitutes absolute fixture paths; it does not replace the production algorithms. The real journey first caught and corrected a literal client-IP placeholder being forwarded upstream.

Full CLI integration, normal push hooks, Linux packaging, provider API execution, certificate installation/renewal, static release switching, backend readiness and restart/recovery remain pending. The separate graph build/configuration delegate owns the preceding foundation. This slice must follow it through full integration before publication. No production cutover or cloud operation was performed.
