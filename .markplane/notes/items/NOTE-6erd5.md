---
id: NOTE-6erd5
title: 'Decision: Auto-managed Java runtimes'
status: draft
type: decision
related: []
tags:
- java
created: 2026-08-10
updated: 2026-08-10
---

# Decision: Auto-managed Java runtimes

## Context

Different Minecraft versions require different Java versions (modern → Java 21, 1.17–1.20 → Java 17, 1.12–1.16 → Java 8/16). Requiring users to install Java themselves is the #1 support burden of community launchers.

## Decision

Auto-download and cache the correct Java runtime per Minecraft version, mirroring the official launcher. Detect system JVMs first and use them when suitable; fall back to downloading from Mojang's runtime manifest (via the version JSON `javaVersion` field) with Adoptium as a secondary source.

## Consequences

- New `Java runtime management` epic added to the roadmap.
- Cache layout: `~/.mc-launcher/java/<major>/...` with checksum verification.
- v1 MVP can launch with system Java only if present; full auto-download targets the Java epic timeline.
