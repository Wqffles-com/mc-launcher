---
id: EPIC-u2ckm
title: Minecraft version management
status: done
priority: critical
started: 2026-08-09
target: 2026-09-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-10
---

# Minecraft version management

## Objective

Make mc-launcher able to install and launch vanilla Minecraft: fetch the Mojang version manifest, download the client jar, libraries, and assets, resolve JVM/game arguments with rules, and spawn the game process. This is the heart of the launcher.

## Key Results

- [x] KR1: Any release and snapshot from the Mojang manifest installs and launches
- [x] KR2: Downloads show progress, resume on interruption, and verify SHA-1
- [x] KR3: Game stdout/stderr captured to log files with clean exit-code reporting

## Notes

- Sources: `piston-meta.mojang.com/mc/game/version_manifest_v2.json` and per-version JSON.
- Natives (LWJGL) must be unpacked per-OS; assets use the asset index (both virtual and non-virtual layouts).
- Depends on Microsoft auth for profile-based launch, but should support offline fallback for debugging.
- Landed 2026-08-10 (TASK-458cd, TASK-ra35r, TASK-wpwuz, TASK-y2n5u, TASK-qc438, TASK-5ny9z):
  - `core/src/download.rs` — resume-able downloads with progress + SHA-1/size verification.
  - `core/src/rules.rs` — platform rules (OS/arch/features), library & native classifier selection, maven coordinates incl. `natives-windows-${arch}` and 4-part classifier names.
  - `core/src/args.rs` — `${token}` expansion, modern `arguments` block and legacy `minecraftArguments`.
  - `core/src/assets.rs` — asset index fetch, content-addressed object store, virtual + legacy materialization.
  - `core/src/launch.rs` — install pipeline (client jar, libraries, natives unpacking, logging config), `build_command` with classpath/natives/assets tokens, offline player profile (v3 UUID), process spawn with stdout/stderr capture to `logs/launcher/<ts>.log` and exit codes.
  - CLI `install` and `launch` (bare version or `--instance`, `--username`, `--memory`, `--java`, `--width/--height`).
- Verified live: 1.8.9 (legacy args, maven fallback, `${arch}` natives), 1.21.4 (modern args, virtual assets), 26.3-snapshot-7 (4-part classifier natives) install; 1.21.4 and snapshot launched to the title screen on Windows with Java 21.
- Launch uses an offline profile until TASK-ix345 (Microsoft auth) lands; Java runtime selection is system Java until EPIC-8g7y2.
