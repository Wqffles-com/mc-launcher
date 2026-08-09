---
id: TASK-tb94s
title: Instance config persistence
status: done
priority: high
type: feature
effort: medium
epic: EPIC-65jcv
plan: null
depends_on: [TASK-iv2v4]
blocks: [TASK-p8cc4, TASK-8v6kj, TASK-wy4n7]
related: []
assignee: null
tags: []
position: a9
created: 2026-08-09
updated: 2026-08-10
---

# Instance config persistence

## Description

Persist per-instance configuration in a human-readable, diff-friendly format inside the instance folder so instances survive restarts and are portable.

## Acceptance Criteria

- [x] `InstanceConfig` (id, name, version, optional loader, game dir, created/last-played timestamps) persisted as pretty JSON in `instances/<id>/instance.json`.
- [x] Config reloads from disk (list/get round-trip) and writes are atomic (temp file + rename).
- [x] Timestamps are RFC 3339 UTC strings (readable, sorted-friendly), produced by a dependency-free clock helper.

## Notes

- Landed in `core/src/instances.rs` + `core/src/clock.rs`. Schema versioning deferred until the modpack lockfile epic.

## References
