---
id: EPIC-5ypvw
title: CurseForge integration
status: later
priority: medium
started: null
target: 2027-03-31
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# CurseForge integration

## Objective

Integrate CurseForge: search and install mods and modpacks from CurseForge's API. Many legacy and popular mods live only on CurseForge, so parity with Modrinth is required for a complete launcher.

## Key Results

- [ ] KR1: Search → install → launch loop works for a representative mod set
- [ ] KR2: CurseForge modpacks install with correct loader/version mappings
- [ ] KR3: API key handled securely and documented for self-hosting

## Notes

- API: `api.curseforge.com/v1` — requires a user API key (CFCore); design key management in settings.
- Modpack format: zip with manifest.json (projectID/fileID references) — no dependency metadata in the manifest; resolves via project files.
- Watch licensing/ToS — CurseForge has stricter redistribution rules than Modrinth.
