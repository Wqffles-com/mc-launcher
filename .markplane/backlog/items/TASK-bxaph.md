---
id: TASK-bxaph
title: Fabric loader install (fabric-meta API)
status: done
priority: high
type: feature
effort: large
epic: EPIC-s7arr
plan: null
depends_on: [TASK-wy4n7]
blocks: [TASK-nh934]
related: []
assignee: null
tags: []
position: aB
created: 2026-08-09
updated: 2026-08-11
---

# Fabric loader install (fabric-meta API)

## Description

Install Fabric loader profiles into the launcher so instances can run
modded Minecraft: query the fabric-meta API (`meta.fabricmc.net/v2`) for
supported game versions and per-game loader versions, fetch and cache the
per-combination launcher profile (`profile/json` — a version JSON that
`inheritsFrom` the game version and adds KnotClient as main class, the
loader's libraries from `maven.fabricmc.net` and its JVM args), and merge
it with the game's version JSON for install and launch.

## Acceptance Criteria

- [x] `fabric list [game]` lists supported games or the loaders for a game (version, stable flag, maven coordinate)
- [x] `fabric install <game> [--loader <version>]` installs the merged profile (client, libraries incl. loader libs from `maven.fabricmc.net`, natives, assets), defaulting to the newest stable loader
- [x] Profiles are cached at `cache/loaders/fabric/<game>-<loader>.json` and reused without a network request
- [x] `instance set-loader fabric <version>` validates the loader version against the API
- [x] `launch --instance` on an instance with a Fabric loader resolves, merges and launches the loader profile
- [x] Merging replaces game libraries the loader pins by `group:artifact` (e.g. the loader's ASM over the game's), avoiding duplicate-ASM classpath failures
- [x] Unknown games/loader versions produce actionable errors; snapshots with spaces are URL-encoded

## Notes

- Profile JSON shape (v2): `{ id: fabric-loader-<loader>-<game>, inheritsFrom: <game>, mainClass: KnotClient, arguments.jvm: [-DFabricMcEmu=...], libraries: [{name, url, sha1?, size?}] }` — legacy libraries carry a maven base `url` (+ optional own sha1/size) instead of a `downloads` block; `rules::library_file` now falls back to that host (verified against the live API 2026-08-11).
- `merge` order: game libraries first, loader libraries appended, loader args appended to game args; everything else (client jar, assets, Java version, logging) inherited from the game version.
- Live E2E 2026-08-11: `fabric install 1.21.4` downloaded loader 0.19.3 artifacts; launching the instance reached the title screen with `FabricLoader 0.19.3` active and exited cleanly.

## References

- fabric-meta README: https://github.com/FabricMC/fabric-meta
