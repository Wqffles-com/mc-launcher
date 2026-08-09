---
id: EPIC-p672c
title: Modrinth integration
status: later
priority: medium
started: null
target: 2027-02-28
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Modrinth integration

## Objective

Integrate Modrinth: search and install mods into instances and install `.mrpack` modpacks, with dependency resolution and update checks. Modrinth is the modern, open mod ecosystem — the primary mod source for the launcher.

## Key Results

- [ ] KR1: Search → install → launch loop works for a representative mod set
- [ ] KR2: `.mrpack` modpacks install with correct loaders and versions
- [ ] KR3: Installed mods checked for updates and bulk-updatable

## Notes

- API: `api.modrinth.com` (v2) — search, project versions, file hashes; no API key required.
- Respect the API rate limits and cache aggressively.
- Modpack install overlaps with the Modpack management epic — coordinate dependency ordering.
