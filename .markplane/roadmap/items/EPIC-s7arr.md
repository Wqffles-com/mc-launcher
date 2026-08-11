---
id: EPIC-s7arr
title: Mod loader support
status: next
priority: high
started: 2026-08-11
target: 2026-11-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-11
---

# Mod loader support

## Objective

Support installing Fabric, Quilt, Forge, and NeoForge loaders into instances and launch loader profiles correctly (merged libraries, transformed args, loader-specific tweakers/entrypoints). This is the "modded launcher" promise and unlocks the Modrinth/CurseForge epics.

## Key Results

- [ ] KR1: Fabric + Quilt install and launch on a supported MC version
- [ ] KR2: Forge + NeoForge install and launch (installer processing included)
- [x] KR3: Loader + vanilla versions selectable per instance and combined with version management

## Notes

- Meta APIs: `meta.fabricmc.net`, `meta.quiltmc.org`; Forge/NeoForge use their installer JARs (headless install).
- Loader install mutates the version JSON (libraries, mainClass, args) — keep a clean copy to allow switching loaders.
- Fabric (TASK-bxaph) is done: fabric-meta v2 client, profile fetch/cache at `cache/loaders/fabric/`, merge of `inheritsFrom` profiles (loader libraries + args over the game version, `group:artifact` dedupe for pinned libs), `fabric list|install` CLI, `set-loader` validation, loader-aware launch. Launches verified live on 1.21.4 + loader 0.19.3.

## Progress

- [x] TASK-bxaph: Fabric loader install (fabric-meta API)
- [ ] TASK-jtyvg: Quilt loader install (quilt-meta API)
- [ ] TASK-tbdp5: Forge install (installer processing)
- [ ] TASK-r5f57: NeoForge install
- [ ] TASK-nh934: Loader-aware launch (merged libraries & args)
