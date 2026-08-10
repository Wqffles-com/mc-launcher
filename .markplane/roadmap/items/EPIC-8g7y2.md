---
id: EPIC-8g7y2
title: Java runtime management
status: done
priority: high
started: 2026-08-10
target: 2026-11-30
related: [PLAN-r4vnn]
tags: []
created: 2026-08-09
updated: 2026-08-10
---

# Java runtime management

## Objective

Make Java "just work": detect system JVMs, and auto-download the correct Java runtime for each Minecraft version (modern versions need Java 21, old versions Java 8/17) — mirroring the official launcher's behavior. Users should never have to install Java.

## Key Results

- [x] KR1: System Java detection finds valid JVMs across all 3 OSes
- [x] KR2: Version-appropriate runtime auto-downloads and caches
- [x] KR3: Launch uses the best available runtime with no user intervention

## Notes

- Mojang publishes runtime manifests per version JSON (`javaVersion` field); fallback source: Adoptium API.
- Cache under global dir `~/.mc-launcher/java/<major>/...`; verify checksums before use.
- Landed 2026-08-10 (PLAN-r4vnn): `core/src/java.rs` — `detect_system` (JAVA_HOME, PATH, well-known roots per OS; `release`-file major probing with `java -version` fallback), Mojang per-file runtime downloads (product + component manifests, `raw` file URLs, 16-way concurrent with SHA-1 verification, staging-dir atomic rename into `java/<major>/`), Adoptium single-archive fallback (SHA-256 verified, zip/tar.gz extraction) when a component is absent or its current major mismatches (e.g. `java-runtime-alpha` is Java 16 today, so Java 8 requests fall back), and `resolve_runtime` selection (explicit `--java` → system exact major → managed auto-download → nearest system JVM). `launch()` resolves the runtime before installing; CLI gains `mc-launcher java list|install`. Verified live: Java 21 installed from Mojang (~200 MB, 402 files) and launched via Adoptium fallback; system JDK 21 detected. Bug found & fixed along the way: `.part` download paths collided for `java.dll`/`java.exe` pairs (extension-replacing suffix) — `download::partial_path` now appends `.part` to the file name.
