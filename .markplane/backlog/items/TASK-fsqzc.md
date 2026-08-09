---
id: TASK-fsqzc
title: Instance import/export
status: done
priority: low
type: enhancement
effort: medium
epic: EPIC-65jcv
plan: null
depends_on: []
blocks: []
related: []
assignee: null
tags: []
position: a0
created: 2026-08-09
updated: 2026-08-10
---

# Instance import/export

## Description

Package an instance (config + game dir) into a portable ZIP archive and restore it on the same or another machine.

## Acceptance Criteria

- [x] `export` writes a ZIP of `instance.json` + `game/` to `exports/<name>-<id>.zip` or an explicit path; deflate compressed.
- [x] `import` restores the archive into a new instance, keeping the archived id when free and reassigning when taken; name override supported.
- [x] Archives are treated as untrusted: entries are sanitized (no `..` / absolute / drive-letter paths), archived ids must match the `in-<16 hex>` shape or are replaced, archived names are validated and must not collide, and extraction is streamed with per-entry/total size limits (zip-bomb protection).
- [x] CLI: `instance export <instance> [-o path]`, `instance import <file> [--name <name>]`.

## Notes

- Landed in `core/src/instances.rs` with the `zip` crate; round-trip and traversal-rejection covered by tests.

## References
