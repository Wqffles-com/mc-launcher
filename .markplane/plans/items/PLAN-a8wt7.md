---
id: PLAN-a8wt7
title: v1 launch pipeline implementation
status: done
implements: [TASK-y2n5u]
related: []
created: 2026-08-09
updated: 2026-08-10
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

- **Phase 1 - Models**: manifest + version JSON serde types with rules evaluation. **DONE (2026-08-10)**: TASK-pam7q (manifest fetch/cache, `core/src/version_manifest.rs`) and TASK-2vzm2 (version JSON models, `core/src/version_json.rs`) landed; CLI `version list/info` works against the live Mojang API.
- **Phase 2 - Downloads**: concurrent library downloader, asset index (virtual + non-virtual). **DONE (2026-08-10)**: TASK-458cd (`core/src/download.rs`, TASK-ra35r (`core/src/assets.rs`) landed; verified live on 1.8.9 (maven fallback URLs, `natives-windows-${arch}`), 1.21.4, and a 2026 snapshot (4-part classifier coordinates, ~5k assets).
- **Phase 3 - Launch**: argument merging, natives unpacking, process spawn, log piping, exit codes. **DONE (2026-08-10)**: TASK-wpwuz (`core/src/rules.rs`, `core/src/args.rs`), TASK-y2n5u + TASK-qc438 (`core/src/launch.rs`, offline v3-UUID profile, log capture to `logs/launcher/`, exit-code propagation) landed; 1.21.4 launched to the title screen on Windows with Java 21.

## Testing

- Unit tests for rules/argument resolution and manifest parsing (fixtures checked in).
- Integration test launching an old release offline with a system JVM.
- markplane check keeps this plan linked to its tasks.

## Rollback

Core is a library - keep each phase on a feature flag; the CLI only exposes the pipeline when Phase 3 lands.

## Retrospective

- `git log`-style record: the pipeline shipped as a single change; asset duplicate hashes and 4-part/`${arch}` maven coordinates were only caught by live installs — worth adding fixture coverage if regressing.
