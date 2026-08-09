---
id: EPIC-65jcv
title: Instances & profiles
status: done
priority: high
started: 2026-08-10
target: 2026-10-31
related: []
tags: []
created: 2026-08-09
updated: 2026-08-10
---

# Instances & profiles

## Objective

Give users isolated instances: each instance has its own game directory, chosen Minecraft version (and later loader), and persisted configuration. This is what separates a launcher from a one-shot downloader and is the core of the v1 MVP.

## Key Results

- [x] KR1: Instance create/delete/clone works with no shared state between instances
- [x] KR2: Per-instance version selection drives install and launch
- [x] KR3: Config persists across restarts and is human-readable

## Notes

- Instance config as JSON/YAML in the instance folder, versioned-friendly for export later.
- Shared artifacts (client jar, libraries, assets) stay global; only instance-specific files live per-instance.
- Landed 2026-08-10: `InstanceManager` in `core/src/instances.rs` with create/delete/clone, per-instance version & loader selection, isolated `game/` dirs, and ZIP import/export; `config.json` launcher schema with directory overrides (`core/src/config.rs`); full `mc-launcher instance` CLI.
- Launch-side wiring (passing `--gameDir`) is covered by TASK-y2n5u in the launch epic, which consumes `Instance::game_dir()`.
