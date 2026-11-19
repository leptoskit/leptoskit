# leptoskit_cloud

The shared types of LeptosKit Cloud, the hosted platform that runs any leptoskit app from the `leptoskit.toml` it already has. The CLI (`lk cloud`), the dashboard, the control plane, and the build and cluster workers all speak them.

- `slug`: the validated names organisations, projects, and environments go by (`[a-z][a-z0-9-]{1,38}[a-z0-9]`, no `--`, a few words reserved), and `Slug::suggest`, which turns `"My Notes App"` into `my-notes-app`.
- `size`: process sizes (`xs` to `xl`) with their CPU and memory, replica scale (`min`, `max` up to 20), sleeping when idle, database sizes, backups, and regions. They are defined once, in `leptoskit_config::cloud`, because the `[cloud]` table parses them.
- `deployment`: a deployment's states and the one function that allows or refuses each transition, the stable code for why one failed (`build_failed`, `spend_cap_reached`, …), and where its source came from (an upload, a Git commit, a template, or a rollback).
- `archive`: the source archive. With the `archive` feature, `pack` turns a project into a deterministic gzip-compressed tar (Git's file list, or a walk that honours `.gitignore`, plus `.leptoskitignore`), refusing secret files, oversized files, and links that leave the project; `unpack` writes one into an empty directory, refusing anything that could land outside it, and runs no program.
- `pricing`: a price catalogue (plans and per-meter unit prices, which are the operator's data, never constants here), and the monthly estimate for a `[cloud]` table, in integer minor units with every assumption stated.

Without features the crate has no server dependencies and builds for `wasm32-unknown-unknown`; `tests/dependencies.rs` keeps it that way.

The cloud's tables, operations, server, and workers come in later releases. Nothing in this crate talks to a network, and no cloud runs yet.
