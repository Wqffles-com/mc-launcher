---
id: EPIC-8g7y2
title: Java runtime management
status: next
priority: high
started: null
target: 2026-11-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Java runtime management

## Objective

Make Java "just work": detect system JVMs, and auto-download the correct Java runtime for each Minecraft version (modern versions need Java 21, old versions Java 8/17) — mirroring the official launcher's behavior. Users should never have to install Java.

## Key Results

- [ ] KR1: System Java detection finds valid JVMs across all 3 OSes
- [ ] KR2: Version-appropriate runtime auto-downloads and caches
- [ ] KR3: Launch uses the best available runtime with no user intervention

## Notes

- Mojang publishes runtime manifests per version JSON (`javaVersion` field); fallback source: Adoptium API.
- Cache under global dir `~/.mc-launcher/java/<major>/...`; verify checksums before use.
