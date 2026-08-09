---
id: EPIC-ku4c7
title: Release, packaging & updates
status: later
priority: medium
started: null
target: 2027-06-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Release, packaging & updates

## Objective

Take the launcher from working software to a shipped product: v1.0 beta milestone and feature freeze, installers for all three platforms, an auto-update mechanism, user documentation, and crash reporting. This is the definition of "done" for v1.

## Key Results

- [ ] KR1: v1.0 released on GitHub Releases with signed installers on all 3 OSes
- [ ] KR2: Auto-update delivers a real patch end to end
- [ ] KR3: README + usage docs cover install, login, instances, mods

## Notes

- Signing: self-signed/CI certs on Windows (or document SmartScreen warning), notarization story on macOS.
- Update channel: GitHub Releases API — check, download, swap, relaunch.
- Blocked on core+app features; this epic gates the 1.0 tag.
