---
id: TASK-2vzm2
title: Parse version JSON (client, libraries, assets, arguments, rules)
status: done
priority: critical
type: feature
effort: medium
epic: EPIC-u2ckm
plan: null
depends_on: [TASK-pam7q]
blocks: [TASK-ra35r, TASK-458cd]
related: []
assignee: null
tags: []
position: a1
created: 2026-08-09
updated: 2026-08-10
---

# Parse version JSON (client, libraries, assets, arguments, rules)

## Description

Parse the per-version JSON into typed serde models: downloads (client/server/mappings), libraries (artifact, classifiers, natives, extract, rules), asset index, modern `arguments` (game/jvm, plain + rules-gated) and legacy `minecraftArguments`, java version, logging, and minimum launcher version.

## Acceptance Criteria

- [x] `core::version_json` models cover the full schema incl. rules (action, os, features) and untagged plain/ruled arguments
- [x] Both modern (1.21.4-style) and legacy (1.8.9-style) fixtures parse; invalid JSON rejected
- [x] `mc-launcher version info <id>` prints a parsed summary from the live Mojang API (verified: 26.2, 131 libraries, 26 game/13 jvm args)

## Notes

- Implemented in `core/src/version_json.rs`. Rules evaluation is deferred to TASK-wpwuz (resolution step).
- Client download for 26.2: 39,193,383 bytes, java major 25.

## References
