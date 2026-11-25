# syntax=docker/dockerfile:1

FROM moby/buildkit:v0.19.0-rootless AS buildkit

FROM rustlang/rust:nightly-2026-10-21 AS worker-builder
ENV RUSTUP_TOOLCHAIN=nightly-2026-10-21 \
	CARGO_NET_RETRY=10
WORKDIR /src

RUN apt-get update \
	&& apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev cmake clang \
	&& rm -rf /var/lib/apt/lists/*

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
	--mount=type=cache,target=/src/target \
	CARGO_BUILD_JOBS=4 cargo build --release --locked \
		-p leptoskit_cloud_worker --bin leptoskit-cloud-worker \
	&& cp /src/target/release/leptoskit-cloud-worker /usr/local/bin/leptoskit-cloud-worker

FROM worker-builder AS cli-builder
RUN --mount=type=cache,target=/usr/local/cargo/registry \
	--mount=type=cache,target=/src/target \
	CARGO_BUILD_JOBS=4 cargo build --release --locked \
		-p leptoskit_cli --bin lk \
	&& cp /src/target/release/lk /usr/local/bin/lk

FROM worker-builder AS notifications-builder
RUN --mount=type=cache,target=/usr/local/cargo/registry \
	--mount=type=cache,target=/src/target \
	CARGO_BUILD_JOBS=4 cargo build --release --locked \
		-p leptoskit_cloud_notifications --bin leptoskit-cloud-notifications \
	&& cp /src/target/release/leptoskit-cloud-notifications /usr/local/bin/leptoskit-cloud-notifications

# Minimal image for the Kubernetes cluster worker role (no build tooling).
FROM debian:trixie-slim AS cluster
RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin leptoskit

COPY --from=worker-builder /usr/local/bin/leptoskit-cloud-worker /usr/local/bin/leptoskit-cloud-worker

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/leptoskit-cloud-worker"]
CMD ["run"]

# A cluster's notifications hub. It pairs itself from
# `LEPTOSKIT_CLOUD_NOTIFICATIONS_PAIRING_CODE`; the worker binary is here
# for an operator who pairs it by hand.
FROM debian:trixie-slim AS notifications
RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin leptoskit

COPY --from=worker-builder /usr/local/bin/leptoskit-cloud-worker /usr/local/bin/leptoskit-cloud-worker
COPY --from=notifications-builder /usr/local/bin/leptoskit-cloud-notifications /usr/local/bin/leptoskit-cloud-notifications

USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/leptoskit-cloud-notifications"]

# Full image for the build worker role (includes `lk` CLI and rootless BuildKit).
FROM debian:trixie-slim AS builder
RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates git uidmap \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --uid 1000 --user-group --create-home --shell /bin/sh buildkit

COPY --from=worker-builder /usr/local/bin/leptoskit-cloud-worker /usr/local/bin/leptoskit-cloud-worker
COPY --from=cli-builder /usr/local/bin/lk /usr/local/bin/lk
COPY --from=buildkit /usr/bin/buildctl /usr/local/bin/buildctl
COPY --from=buildkit /usr/bin/buildkitd /usr/local/bin/buildkitd
COPY --from=buildkit /usr/bin/rootlesskit /usr/local/bin/rootlesskit
COPY --from=buildkit /usr/bin/buildctl-daemonless.sh /usr/local/bin/buildctl-daemonless.sh

USER 1000:1000
ENV LEPTOSKIT_CLOUD_BUILDER_BACKEND=buildctl
ENTRYPOINT ["/usr/local/bin/leptoskit-cloud-worker"]
CMD ["run"]
