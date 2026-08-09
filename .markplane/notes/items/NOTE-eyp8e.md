---
id: NOTE-eyp8e
title: 'Decision: All releases + snapshots support'
status: draft
type: decision
related: []
tags:
- versions
created: 2026-08-10
updated: 2026-08-10
---

# Decision: All releases + snapshots support

## Context

Users of community launchers play everything from 1.7.10 modpacks to the latest snapshot. Restricting to latest releases would break modded usage and early testing.

## Decision

Support the full Mojang version manifest: all releases and snapshots, plus experimental versions where the manifest exposes them. Custom/manual version JSON import is a later enhancement.

## Consequences

- Rules evaluation (OS/feature rules) must be correct for legacy versions (pre-1.13 argument format vs. the modern `arguments` object).
- Downloader must handle legacy asset index formats.
- Test matrix includes 1.8.9 (legacy args), 1.12.2 (modded staple), 1.16.5, 1.20.x, and latest release + snapshot.
