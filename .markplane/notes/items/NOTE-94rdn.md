---
id: NOTE-94rdn
title: 'Research: Modrinth & CurseForge API landscape'
status: draft
type: research
related: []
tags:
- mods
created: 2026-08-10
updated: 2026-08-10
---

# Research: Modrinth & CurseForge API landscape

## Summary

Modrinth and CurseForge are the two mod ecosystems a modern launcher must support. They differ fundamentally: Modrinth is open and keyless; CurseForge requires a user API key and has tighter redistribution rules.

## Findings

- **Modrinth API v2** (`api.modrinth.com`): public, no key; endpoints for search (`/v2/search`), project versions, version files (sha1/sha512 hashes), and `.mrpack` modpacks (zip with `modrinth.index.json` declaring dependencies + optional files). Rate limits ~300 req/5min; `User-Agent` required.
- **CurseForge API v1** (`api.curseforge.com/v1`): requires an API key (`x-api-key` header) — users must generate one and paste it (no default launcher key). Modpack format is a zip with `manifest.json` referencing `projectID` + `fileID`; optional overrides folder; no dependency metadata in the manifest — must resolve via project file APIs.
- License/ToS: Modrinth explicitly permits launcher use; CurseForge disallows re-distribution of mod files — downloads must be direct from their CDN.

## Recommendations

- Modrinth first (no key friction), CurseForge second with a settings UI for the API key.
- Both installers write to the instance's `mods/` folder; unify behind a `ModInstaller` trait.
- Coordinate with Modpack management epic for import/export of both formats.

## References

- [[EPIC-p672c]] [[EPIC-5ypvw]] [[EPIC-9x4rg]]
