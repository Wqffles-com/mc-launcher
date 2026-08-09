---
id: EPIC-9n3wy
title: Project foundation & tooling
status: now
priority: high
started: 2026-08-09
target: 2026-08-31
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Project foundation & tooling

## Objective

Set up the Cargo workspace (`core` library, `cli` binary, `app` Tauri target), CLI skeleton, config schema, and CI pipeline so every later epic builds on a stable, tested base. Everything downstream (versions, auth, instances, mods) depends on this foundation.

## Key Results

- [ ] KR1: Workspace builds cleanly on Windows, macOS, and Linux
- [ ] KR2: `mc-launcher --help` exposes init/install/launch/login/instance subcommands
- [ ] KR3: CI runs lint + tests + cross-platform build on every push

## Notes

- Rust edition 2024, clap v4 for the CLI, tokio + reqwest for async networking.
- Directory layout: `~/.mc-launcher/` for global state (downloads, java, accounts), instances under `instances/`.
- MIT license, public repo `mc-launcher`.
