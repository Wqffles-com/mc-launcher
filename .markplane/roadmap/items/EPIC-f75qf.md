---
id: EPIC-f75qf
title: Desktop app (Tauri + React)
status: next
priority: high
started: null
target: 2026-12-31
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Desktop app (Tauri + React)

## Objective

Ship the desktop app: a Tauri v2 application with a React + TypeScript frontend styled with shadcn/ui + Tailwind (the "Next.js vibe"), covering version browsing/install, Microsoft login, instance management, play + console view, and settings — all backed by the Rust core via a typed RPC bridge.

## Key Results

- [ ] KR1: Every core feature is reachable from the UI with no raw CLI needed
- [ ] KR2: Launch flow shows progress, live logs, and clean error states
- [ ] KR3: App builds and runs on Windows, macOS, and Linux

## Notes

- Tauri commands expose core as async commands; keep the bridge thin — all logic stays in the `core` crate.
- Design tokens + components from shadcn/ui; dark mode first (gamers), light mode supported.
- First priority after core/CLI stabilization per project decision.
