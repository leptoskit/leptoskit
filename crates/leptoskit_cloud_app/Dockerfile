# syntax=docker/dockerfile:1

FROM rustlang/rust:nightly-bookworm AS builder
WORKDIR /src

RUN apt-get update \
	&& apt-get install -y --no-install-recommends pkg-config libssl-dev cmake clang \
	&& rm -rf /var/lib/apt/lists/* \
	&& rustup target add wasm32-unknown-unknown

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

# Build the production control plane binary (SSR console + Postgres + SQLite)
# and the Wasm hydration bundle served from LEPTOSKIT_CLOUD_SITE.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
	--mount=type=cache,target=/src/target \
	CARGO_BUILD_JOBS=4 cargo build --release --locked \
		-p leptoskit_cloud_app --bin leptoskit-cloud-control --features production \
	&& cp /src/target/release/leptoskit-cloud-control /usr/local/bin/leptoskit-cloud-control \
	&& mkdir -p /srv/site/pkg \
	&& CARGO_BUILD_JOBS=4 cargo build --release --locked \
		--target wasm32-unknown-unknown \
		-p leptoskit_cloud_app --lib --no-default-features --features hydrate \
	&& cp /src/target/wasm32-unknown-unknown/release/leptoskit_cloud_app.wasm /srv/site/pkg/

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates curl \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin leptoskit

COPY --from=builder /usr/local/bin/leptoskit-cloud-control /usr/local/bin/leptoskit-cloud-control
COPY --from=builder --chown=65532:65532 /srv/site /srv/site

USER 65532:65532
ENV LEPTOSKIT_CLOUD_BIND=0.0.0.0:8080 \
	LEPTOSKIT_CLOUD_SITE=/srv/site
EXPOSE 8080

HEALTHCHECK --interval=15s --timeout=5s --start-period=15s --retries=3 \
	CMD ["curl", "-fsS", "http://127.0.0.1:8080/readyz"]

ENTRYPOINT ["/usr/local/bin/leptoskit-cloud-control"]
CMD ["serve"]
