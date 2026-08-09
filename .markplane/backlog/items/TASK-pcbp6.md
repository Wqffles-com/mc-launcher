---
id: TASK-pcbp6
title: CLI skeleton with clap subcommands
status: done
priority: high
type: feature
effort: medium
epic: EPIC-9n3wy
plan: null
depends_on: []
blocks: []
related: []
assignee: null
tags: []
position: a1
created: 2026-08-09
updated: 2026-08-10
---

# CLI skeleton with clap subcommands

## Description

clap v4 skeleton exposing `init`, `install`, `launch`, `login`, `instance` and `version` (list/info) subcommands. `init` and `version` are functional end-to-end; the rest are stubs that land with their epics.

## Acceptance Criteria

- [x] `mc-launcher --help` lists init/install/launch/login/instance/version subcommands
- [x] `mc-launcher init` creates `~/.mc-launcher/` layout
- [x] `mc-launcher version list` and `version info <id>` work against the live Mojang API

## Notes

- Implemented in `cli/src/main.rs` on tokio; anyhow for CLI error reporting.
- Install/launch stubs reference their target tasks (TASK-458cd, TASK-ix345, EPIC-65jcv).

## References
