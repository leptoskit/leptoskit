FROM rustlang/rust:nightly-2026-10-21 AS build
ENV RUSTUP_TOOLCHAIN=nightly-2026-10-21
WORKDIR /repo
COPY . .
RUN CARGO_BUILD_JOBS=4 cargo build --locked --release -p leptoskit_playground_server --bin playground-worker

FROM rustlang/rust:nightly-2026-10-21
ENV RUSTUP_TOOLCHAIN=nightly-2026-10-21
RUN rustup target add wasm32-unknown-unknown && cargo install --locked wasm-bindgen-cli --version 0.2.129
COPY . /repo
COPY crates/leptoskit_playground_server/image/harness /harness
COPY Cargo.lock /harness/Cargo.lock
WORKDIR /harness
RUN CARGO_BUILD_JOBS=4 cargo build --target wasm32-unknown-unknown --message-format=json > /harness/artifacts.json && chmod -R a+rX /harness /repo /usr/local/cargo /usr/local/rustup
COPY --from=build /repo/target/release/playground-worker /usr/local/bin/
WORKDIR /work
USER 65534:65534
ENTRYPOINT ["playground-worker"]
