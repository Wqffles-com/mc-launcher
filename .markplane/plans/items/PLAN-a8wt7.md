---
id: PLAN-a8wt7
title: v1 launch pipeline implementation
status: draft
implements: [TASK-y2n5u]
related: []
created: 2026-08-09
updated: 2026-08-09
---

# v1 launch pipeline implementation

## Overview

Implement the complete vanilla launch pipeline in the Rust core: manifest fetch, version parse, artifact download, argument resolution, and process spawn. This plan is the spine of the v1 MVP.

## Approach

1. Build the version manifest + version JSON models first (TASK-pam7q, TASK-2vzm2).
2. Implement the download manager with progress/resume/SHA-1 verification (TASK-458cd, TASK-ra35r).
3. Resolve rules-based JVM/game arguments and natives (TASK-wpwuz).
4. Spawn the Java process with the resolved command line and capture output (TASK-y2n5u, TASK-qc438).
5. Verify against a matrix: old release (1.8.9), modern release (latest), snapshot, each on the 3 OSes.

## Phases

- **Phase 1 - Models**: manifest + version JSON serde types with rules evaluation.
- **Phase 2 - Downloads**: concurrent library downloader, asset index (virtual + non-virtual).
- **Phase 3 - Launch**: argument merging, natives unpacking, process spawn, log piping, exit codes.

## Testing

- Unit tests for rules/argument resolution and manifest parsing (fixtures checked in).
- Integration test launching an old release offline with a system JVM.
- markplane check keeps this plan linked to its tasks.

## Rollback

Core is a library - keep each phase on a feature flag; the CLI only exposes the pipeline when Phase 3 lands.