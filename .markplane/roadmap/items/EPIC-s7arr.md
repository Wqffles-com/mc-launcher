---
id: EPIC-s7arr
title: Mod loader support
status: next
priority: high
started: null
target: 2026-11-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Mod loader support

## Objective

Support installing Fabric, Quilt, Forge, and NeoForge loaders into instances and launch loader profiles correctly (merged libraries, transformed args, loader-specific tweakers/entrypoints). This is the "modded launcher" promise and unlocks the Modrinth/CurseForge epics.

## Key Results

- [ ] KR1: Fabric + Quilt install and launch on a supported MC version
- [ ] KR2: Forge + NeoForge install and launch (installer processing included)
- [ ] KR3: Loader + vanilla versions selectable per instance and combined with version management

## Notes

- Meta APIs: `meta.fabricmc.net`, `meta.quiltmc.org`; Forge/NeoForge use their installer JARs (headless install).
- Loader install mutates the version JSON (libraries, mainClass, args) — keep a clean copy to allow switching loaders.
