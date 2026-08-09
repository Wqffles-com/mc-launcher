---
id: TASK-pam7q
title: Fetch & cache Mojang version manifest
status: done
priority: critical
type: feature
effort: small
epic: EPIC-u2ckm
plan: null
depends_on: []
blocks: [TASK-2vzm2, TASK-5ny9z]
related: []
assignee: null
tags: []
position: a0
created: 2026-08-09
updated: 2026-08-10
---

# Fetch & cache Mojang version manifest

## Description

Fetch `piston-meta.mojang.com/mc/game/version_manifest_v2.json`, parse it into typed models, and cache it under `~/.mc-launcher/cache/version_manifest_v2.json` with a TTL (default 6h), re-fetching only when stale or forced. Expose query helpers (find by id, latest release/snapshot, filter by type).

## Acceptance Criteria

- [x] `core::version_manifest` fetches and parses the manifest (serde models for `latest`, `versions`)
- [x] Cache is written atomically (tmp + rename) and reused when fresh; `force` re-fetches
- [x] CLI `mc-launcher version list` works against the live manifest, cache hit ~0.3s
- [x] Unit tests cover parse, HTTP errors, fresh-cache hit, stale refetch, forced refetch

## Notes

- Implemented in `core/src/version_manifest.rs` (fetch, load, cache helpers) and surfaced in `cli/src/main.rs`.
- Verified live against Mojang on 2026-08-10 (latest release 26.2).

## References

