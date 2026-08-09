---
id: NOTE-ik4nz
title: 'Research: Mojang launch pipeline & version JSON'
status: draft
type: research
related: []
tags:
- minecraft
created: 2026-08-10
updated: 2026-08-10
---

# Research: Mojang launch pipeline & version JSON

## Summary

Launching Minecraft programmatically requires mirroring what the official launcher does: read `version_manifest_v2.json`, download the version JSON, resolve artifacts (client jar, libraries, natives, assets), evaluate rules, and assemble a JVM command line.

## Findings

- Manifest: `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json` — every release + snapshot with `latest.release` / `latest.snapshot`.
- Version JSON: `downloads.client`, `downloads.libraries` (with `rules`, `natives` maps), `assetIndex` (virtual or non-virtual), `arguments` (modern) vs `minecraftArguments` (legacy, pre-1.13), `mainClass` (with launcher tweakers for legacy), `javaVersion` (modern).
- Libraries live at `libraries.mojang.com` or via mirrors (BMCLAPI in CN); assets hashed under `resources.download.minecraft.net/<sha1-prefix>/<sha1>`.
- Auth tokens go in `--accessToken`, profile as `--uuid`/`--username` (JSONArg / `auth_player_name` etc. substituted).
- Reference implementations: `minecraft-launcher-lib` (Python), `OpenLauncherLib`, PrismLauncher source.

## Recommendations

- Model the full version JSON with serde, keeping unknown fields tolerant (forward compatibility).
- Implement a `Rule` evaluator shared by both argument styles.
- Download manager: sha1 verify + resume; asset download in parallel batches.

## References

- [[TASK-pam7q]] [[TASK-2vzm2]] [[TASK-458cd]] [[TASK-wpwuz]] [[TASK-y2n5u]] [[PLAN-a8wt7]]
