---
id: EPIC-9x4rg
title: Modpack management
status: later
priority: medium
started: null
target: 2027-04-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Modpack management

## Objective

Handle modpacks end to end: import local archives (mrpack/CurseForge zip), export instances as shareable modpacks, and support lockfiles so instances are reproducible. Modpacks are the top feature users expect from a modern modded launcher.

## Key Results

- [ ] KR1: Imported modpacks install and launch without manual fixes
- [ ] KR2: Export → re-import round-trip preserves mods, configs, and loader
- [ ] KR3: Lockfile pins exact mod versions for reproducibility

## Notes

- Builds on the Modrinth and CurseForge epics — their installers are the import path.
- Export needs care with licensing: only include files we're allowed to redistribute, or keep as metadata + re-download (preferred).
