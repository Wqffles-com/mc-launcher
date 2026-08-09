---
id: NOTE-s43ph
title: 'Architecture decision: Rust core + CLI + Tauri desktop app'
status: draft
type: decision
related: []
tags:
- architecture
created: 2026-08-10
updated: 2026-08-10
---

# Architecture decision: Rust core + CLI + Tauri desktop app

## Context

The launcher must run on Windows, macOS, and Linux, ship as a modern-looking desktop app (Next.js/shadcn vibe), expose a CLI, and grow to cover mods, modpacks, and servers.

## Decision

- **Core**: Rust library crate (`core`) — manifest, downloads, auth, launch engine, loader install, Modrinth/CurseForge clients.
- **CLI**: Rust binary (`cli`, `mc-launcher`) — thin wrapper over core for power users and CI.
- **App**: Tauri v2 desktop app (`app`) — React + TypeScript frontend with shadcn/ui + Tailwind; all logic stays in core behind a typed RPC bridge.

## Alternatives

- Electron + Node core: rejected — heavy footprint, weaker systems-level integration for game launching.
- Go + Wails: viable, but Rust's ecosystem for this domain (Minecraft tooling, memory safety for game launching) is stronger.

## Consequences

- One codebase serves CLI, app, and future CI automation.
- Rust is the single implementation language for all game-facing logic; frontend is pure UI.
