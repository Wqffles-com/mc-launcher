---
id: TASK-9ik83
title: Per-version Java selection & caching
status: done
priority: high
type: feature
effort: medium
epic: EPIC-8g7y2
plan: PLAN-r4vnn
depends_on: [TASK-4pd8f]
blocks: [TASK-vaajw]
related: []
assignee: null
tags: []
position: aI
created: 2026-08-09
updated: 2026-08-09
---

# Per-version Java selection & caching

## Description

Choose the right runtime for a launch with zero user intervention: explicit `--java` wins; otherwise a system JVM of the exact required major (from the version JSON `javaVersion`, defaulting to 8 for pre-1.13), then an auto-downloaded managed runtime of that major (cached under `java/<major>/`), then the nearest available system JVM, else a clear error. `launch()` resolves the runtime before installing so failures surface fast, and feeds the executable into `build_command`. Exposes `mc-launcher java list` (system + managed) and `mc-launcher java install <major>`. Implements Phase 3 of PLAN-r4vnn.

## Acceptance Criteria

- [ ] Launch of a version whose major has no system JVM downloads and caches it automatically (KR3: no user intervention)
- [ ] Launch uses a system JVM of the exact major when present, before considering downloads; explicit `--java` always wins
- [ ] `mc-launcher java list` shows detected system JVMs and cached managed runtimes with majors; `mc-launcher java install 21` installs and caches
- [ ] No usable runtime anywhere yields an error that tells the user the fix (`mc-launcher java install <major>`)

## Notes

- Selection policy is the Cross-Plan Contract in PLAN-r4vnn; consumed later by the Tauri app (EPIC-f75qf) and loader installs (EPIC-s7arr).

## References

- PLAN-r4vnn Phase 3
