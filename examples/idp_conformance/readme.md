# idp_conformance

A test server for the OpenID Foundation's conformance suite: `leptoskit_idp` with an HTML sign-in page for one seeded person and an HTML consent page, which the suite's scripted browser drives by selector, served over `https`. It signs in through `leptoskit_auth`'s development sign-in, so it only runs as a debug build.

`.github/workflows/idp-conformance.yml` runs it nightly. To run the same thing locally, with Docker or podman:

```sh
git clone --depth 1 --branch release-v5.3.4 https://gitlab.com/openid/conformance-suite.git ~/conformance-suite
docker run --rm -v ~/conformance-suite:/usr/src/mymaven -v ~/.m2:/root/.m2 -w /usr/src/mymaven \
  maven:3-eclipse-temurin-21 mvn -B -q clean package -DskipTests=true
cargo build -p idp_conformance_example
bash examples/idp_conformance/run-suite.sh ~/conformance-suite target/debug/idp_conformance_example /tmp/conformance-results
```

With podman on macOS, keep the suite under your home directory (the VM shares only that) and point `DOCKER_HOST` at podman's socket. `expected-failures.json` and `expected-skips.json` list what the provider doesn't do on purpose, each with its reason; anything else that fails, warns, or is skipped fails the run.
