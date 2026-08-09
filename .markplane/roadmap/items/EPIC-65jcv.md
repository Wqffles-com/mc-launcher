---
id: EPIC-65jcv
title: Instances & profiles
status: next
priority: high
started: null
target: 2026-10-31
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Instances & profiles

## Objective

Give users isolated instances: each instance has its own game directory, chosen Minecraft version (and later loader), and persisted configuration. This is what separates a launcher from a one-shot downloader and is the core of the v1 MVP.

## Key Results

- [ ] KR1: Instance create/delete/clone works with no shared state between instances
- [ ] KR2: Per-instance version selection drives install and launch
- [ ] KR3: Config persists across restarts and is human-readable

## Notes

- Instance config as JSON/YAML in the instance folder, versioned-friendly for export later.
- Shared artifacts (client jar, libraries, assets) stay global; only instance-specific files live per-instance.
