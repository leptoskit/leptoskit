---
"release": patch
---

# Deploy an app that lives inside a Cargo workspace

`leptoskit generate deploy` now finds the server and client packages through Cargo's workspace metadata, so an app folder with only `leptoskit.toml` and member crates works, including members matched by workspace globs. The generated container uses the Cargo workspace as its build context, builds from the app directory, and copies the workspace target binary, the app's `dist/`, and the nearest workspace or app toolchain file. Compose, Fly, Kubernetes, and managed infrastructure use the same context and `Dockerfile` path.

Inside a workspace, the container ignore policy is written as `Dockerfile.dockerignore` beside the app's `Dockerfile`, so Docker applies it to the workspace context instead of ignoring the app-root `.dockerignore`. It excludes nested build caches, databases, environment files, managed infrastructure plans and state, and Kamal secrets, keeps exclusions already in the workspace and app `.dockerignore` files, and never rewrites those files. Standalone apps keep their existing files and commands, custom generated files stay yours, and a path that would escape the workspace is refused before it becomes a build context.

Nested recipes also accept `LEPTOSKIT_CLI_PATH` as an explicit build argument for a CLI crate inside the workspace, and normalize the container target directory independently of host build settings.
