---
id: TASK-c6bqu
title: Instance create/delete/clone
status: done
priority: high
type: feature
effort: medium
epic: EPIC-65jcv
plan: null
depends_on: [TASK-iv2v4]
blocks: []
related: []
assignee: null
tags: []
position: a7
created: 2026-08-09
updated: 2026-08-10
---

# Instance create/delete/clone

## Description

Full lifecycle for game instances: create a new instance (name + Minecraft version), delete one including its game data, and clone one under a new name. Clones must be fully independent copies, not references to the source.

## Acceptance Criteria

- [x] `InstanceManager::create` makes the instance folder + `game/` dir, persists `instance.json`, rejects invalid or duplicate names.
- [x] `delete` removes the instance folder and game data; unknown instances error cleanly.
- [x] `clone` copies config and game contents into a fresh id/name; mutating the clone never touches the source.
- [x] CLI: `instance create/delete/clone/list/info`.

## Notes

- Landed in `core/src/instances.rs`; ids are random hex (`in-<16 hex>`), folder name = id.
- `mc-launcher instance create <name> --version <id>`; version defaults to the latest release from the cached manifest.

## References
