---
id: TASK-iv2v4
title: Project config schema & directory layout
status: done
priority: high
type: feature
effort: medium
epic: EPIC-9n3wy
plan: null
depends_on: []
blocks: [TASK-c6bqu, TASK-tb94s]
related: []
assignee: null
tags: []
position: a2
created: 2026-08-09
updated: 2026-08-10
---

# Project config schema & directory layout

## Description

Define the launcher-wide config schema (`config.json`) and the full on-disk layout, so later features (instances, downloads, auth, Java) have stable homes. Directory overrides let users relocate heavy state (instances, downloads) off the default data root.

## Acceptance Criteria

- [x] `config.json` schema with optional overrides for `instances_dir`, `downloads_dir`, `accounts_dir`, `java_dir`; relative paths resolve against the data root, absolute paths are used as-is.
- [x] `Directories` exposes every layout path (`cache/`, `downloads/`, `java/`, `accounts/`, `instances/`, `exports/`) and honors config overrides.
- [x] Config persists atomically and round-trips; corrupt config fails loudly instead of silently resetting.

## Notes

- Landed in `core/src/config.rs` (`LauncherConfig`) and `core/src/dirs.rs` (`Directories::discover` loads the config, `ensure_all` creates every directory).

## References
