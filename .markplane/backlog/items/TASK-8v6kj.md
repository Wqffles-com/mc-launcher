---
id: TASK-8v6kj
title: Per-instance version & loader selection
status: done
priority: high
type: feature
effort: medium
epic: EPIC-65jcv
plan: null
depends_on: [TASK-tb94s]
blocks: []
related: []
assignee: null
tags: []
position: a8
created: 2026-08-09
updated: 2026-08-10
---

# Per-instance version & loader selection

## Description

Each instance carries its own Minecraft version and (optionally) a mod loader choice, persisted in its config and changeable at any time. Selection is recorded now; loader installation itself is the loader epic.

## Acceptance Criteria

- [x] `InstanceManager::set_version` persists the version; CLI validates it against the Mojang manifest and errors on unknown ids.
- [x] `set_loader`/`clear_loader` persist a `Loader { kind, version }` (`fabric|quilt|forge|neoforge`) — human-readable in `instance.json`.
- [x] `touch` records `last_played_at` for the launch flow.
- [x] CLI: `instance set-version <instance> <id>`, `instance set-loader <instance> <kind> <version>`.

## Notes

- Landed in `core/src/instances.rs`. Loader version strings are validated against loader APIs in the loader-install epic.

## References
