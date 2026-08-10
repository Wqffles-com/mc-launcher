---
id: TASK-4pd8f
title: Java runtime download (Mojang/Adoptium manifest)
status: done
priority: high
type: feature
effort: large
epic: EPIC-8g7y2
plan: PLAN-r4vnn
depends_on: [TASK-28y9j]
blocks: [TASK-9ik83]
related: []
assignee: null
tags: []
position: aH
created: 2026-08-09
updated: 2026-08-10
---

# Java runtime download (Mojang/Adoptium manifest)

## Description

Download and cache a complete Java runtime for a given major version, mirroring the official launcher. Primary source: Mojang's Java runtime manifests — the pinned product manifest (`launchermeta.mojang.com/v1/products/java-runtime/<hash>/all.json`) resolves an OS key + component (`jre-legacy`, `java-runtime-alpha`, `java-runtime-gamma`, `java-runtime-delta`, `java-runtime-epsilon`) to a component manifest of individual files, each downloaded with SHA-1 + size verification and concurrency (16-way) into a staging dir, then atomically renamed to `java/<major>/`. The component is only used when its manifest entry's version matches the requested major (e.g. 1.16.5's `java-runtime-alpha` now points at Java 16, so Java 8 falls through). Fallback: Adoptium's assets API single archive (zip/tar.gz) with SHA-256 verification. Cached runtimes are reused without a request. Implements Phase 2 of PLAN-r4vnn.

## Acceptance Criteria

- [ ] `ensure_runtime(dirs, client, major, component, progress)` installs a Mojang runtime via per-file raw downloads; every file SHA-1 verified; executable bits applied; staging dir atomically renamed into `java/<major>/`
- [ ] Mojang path falls back to Adoptium when the component is absent, mismatched, or unavailable for the platform; archive SHA-256 verified before extraction
- [ ] An existing complete `java/<major>/` runtime is returned without any network request (cache hit)
- [ ] Interrupted downloads never leave a half-installed `java/<major>/` (staging cleanup on error)

## Notes

- All Mojang files expose `raw` downloads, so no LZMA support is needed.
- New deps: `flate2` (rust_backend), `tar`, `sha2`; `download::sha256_file` and `launch::sanitize_entry_path` become `pub(crate)`.

## References

- PLAN-r4vnn Phase 2; Mojang manifests verified against live API 2026-08-10
