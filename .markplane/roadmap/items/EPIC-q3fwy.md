---
id: EPIC-q3fwy
title: Server management
status: later
priority: low
started: null
target: 2027-12-31
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Server management

## Objective

Extend mc-launcher to server hosting: download vanilla server jars, launch and manage servers (console, start/stop), edit `server.properties` and whitelist, and support loader-based servers. Positioned after v1.0 as the flagship post-release feature.

## Key Results

- [ ] KR1: Vanilla server downloads, starts, and accepts console commands
- [ ] KR2: server.properties + whitelist/ops editable from CLI and app
- [ ] KR3: Modded servers (Fabric/Forge) install and run

## Notes

- Reuses version manifest + Java management and (for modded) the loader machinery.
- EULA acceptance flow needed (`eula.txt`) — clear UX.
- Scope is deliberately open; re-plan before starting work.
