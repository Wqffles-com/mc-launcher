---
id: TASK-wy4n7
title: Instance isolation (per-instance game dirs)
status: done
priority: high
type: feature
effort: large
epic: EPIC-65jcv
plan: null
depends_on: [TASK-tb94s, TASK-y2n5u]
blocks: [TASK-tbdp5, TASK-jtyvg, TASK-r5f57, TASK-bxaph, TASK-gz3iu]
related: []
assignee: null
tags: []
position: aA
created: 2026-08-09
updated: 2026-08-10
---

# Instance isolation (per-instance game dirs)

## Description

Each instance owns an isolated game directory (`instances/<id>/game/`: saves, mods, logs, config) so instances never share world/mod state. Shared artifacts (client jar, libraries, assets) remain global in `downloads/`.

## Acceptance Criteria

- [x] `Instance::game_dir()` resolves the per-instance game dir (configurable `game_dir` key, default `game/`), created on instance create.
- [x] Clone copies game contents into a separate directory; changes on either side never leak.
- [x] Import/export archives carry the game dir and restore isolation.
- [x] Launch engine integration (passing `--gameDir`) is wired to `Instance::game_dir()` via TASK-y2n5u (launch epic).

## Notes

- Landed in `core/src/instances.rs`. The launch pipeline (TASK-y2n5u) consumes `game_dir()` as the `--gameDir` value; until it lands, no game process is spawned.

## References
