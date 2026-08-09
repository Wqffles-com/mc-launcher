---
id: TASK-nex24
title: Scaffold Cargo workspace (core, cli, app crates)
status: done
priority: high
type: chore
effort: medium
epic: EPIC-9n3wy
plan: null
depends_on: []
blocks: []
related: []
assignee: null
tags: []
position: a0
created: 2026-08-09
updated: 2026-08-10
---

# Scaffold Cargo workspace (core, cli, app crates)

## Description

Create the Cargo workspace with `core` (library) and `cli` (binary) crates, shared workspace dependencies and lints (edition 2024, forbid unsafe, clippy pedantic). The `app` Tauri crate is scaffolded later with TASK-yeggv.

## Acceptance Criteria

- [x] Workspace builds cleanly with zero warnings (cargo build --workspace)
- [x] `cargo clippy --workspace --all-targets` clean; `cargo fmt --check` clean
- [x] Shared deps (reqwest/rustls, tokio, serde, clap, anyhow, thiserror) declared once in `[workspace.dependencies]`

## Notes

- Structure: root `Cargo.toml` (workspace), `core/` mc-launcher-core, `cli/` mc-launcher.
- CI pipeline is TASK-jrvwv (still open). App crate deferred to TASK-yeggv.

## References
