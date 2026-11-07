FROM rustlang/rust:nightly-2026-10-21 AS build
ENV RUSTUP_TOOLCHAIN=nightly-2026-10-21
WORKDIR /repo
COPY . .
RUN CARGO_BUILD_JOBS=4 cargo build --locked --release -p leptoskit_playground_server --bins

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates docker-cli && rm -rf /var/lib/apt/lists/*
COPY --from=build /repo/target/release/leptoskit_playground_server /usr/local/bin/
ENV PLAY_BIND=0.0.0.0:4702
ENV PLAY_ORIGINS=https://leptoskit.dev
EXPOSE 4702
ENTRYPOINT ["leptoskit_playground_server"]
