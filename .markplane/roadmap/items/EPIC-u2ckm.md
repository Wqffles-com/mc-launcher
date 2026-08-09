---
id: EPIC-u2ckm
title: Minecraft version management
status: now
priority: critical
started: 2026-08-09
target: 2026-09-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Minecraft version management

## Objective

Make mc-launcher able to install and launch vanilla Minecraft: fetch the Mojang version manifest, download the client jar, libraries, and assets, resolve JVM/game arguments with rules, and spawn the game process. This is the heart of the launcher.

## Key Results

- [ ] KR1: Any release and snapshot from the Mojang manifest installs and launches
- [ ] KR2: Downloads show progress, resume on interruption, and verify SHA-1
- [ ] KR3: Game stdout/stderr captured to log files with clean exit-code reporting

## Notes

- Sources: `piston-meta.mojang.com/mc/game/version_manifest_v2.json` and per-version JSON.
- Natives (LWJGL) must be unpacked per-OS; assets use the asset index (both virtual and non-virtual layouts).
- Depends on Microsoft auth for profile-based launch, but should support offline fallback for debugging.
