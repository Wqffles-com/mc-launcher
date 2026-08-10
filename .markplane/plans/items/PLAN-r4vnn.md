---
id: PLAN-r4vnn
title: Java runtime management implementation
status: done
implements: [TASK-28y9j, TASK-4pd8f, TASK-9ik83]
related: [EPIC-8g7y2]
created: 2026-08-10
updated: 2026-08-10
---

# Java runtime management Implementation Plan

## Overview

Make Java "just work" (EPIC-8g7y2): detect system JVMs, auto-download the correct runtime per Minecraft version, cache under `java/<major>/`, and make launch pick the best runtime with zero user intervention. New `core/src/java.rs` module + `mc-launcher java list|install` CLI.

## Ground Truth

- `core/src/launch.rs:410-436` — `resolve_java`: current system-Java lookup (configured → `JAVA_HOME` → PATH). Replaced by the new selector, but kept as the fallback resolver inside `build_command` (`core/src/launch.rs:524`).
- `core/src/launch.rs:543-605` — `launch()` pipeline; new runtime resolution plugs in before `build_command`.
- `core/src/version_json.rs:107-112` — `JavaVersion { component, major_version }`; versions without it (pre-1.13) need Java 8.
- `core/src/download.rs:56-167` — `fetch()` with `.part` resume + SHA-1 verification; reuse for all runtime downloads.
- `core/src/download.rs:180-183` — `sha1_file` pattern; add `sha256_file` for Adoptium's SHA-256 checksums.
- `core/src/launch.rs:390-402` — `sanitize_entry_path` + `extract_native_jar` ZIP extraction pattern (make `sanitize_entry_path` `pub(crate)` and reuse).
- `core/src/error.rs` — error variants; add `JavaRuntime(String)`.
- `cli/src/main.rs:371-469` — `cmd_launch`; `cli/src/main.rs:529-548` — `print_progress` reused for runtime downloads.
- Mojang runtime API (verified 2026-08-10): product manifest `launchermeta.mojang.com/v1/products/java-runtime/<pinned>/all.json` → `{<os-key>: {<component>: [{manifest:{url}, version:{name}}]}}`; component manifest → `{files: {<path>: {type: directory|file, downloads: {raw|...}, executable}}}`. All files have `raw` URLs (LZMA optional, not used). OS keys: `windows-x64|windows-arm64|windows-x86|mac-os|mac-os-arm64|linux|linux-arm64|linux-i386`.
- Adoptium fallback: `api.adoptium.net/v3/assets/latest/<major>/hotspot?architecture=&image_type=jre&os=&vendor=eclipse` → `[{binary.package.{link,name,checksum}}]` (SHA-256).

## Approach

1. **Detection** (`detect_system`): candidates from `JAVA_HOME`, PATH entries containing `java`, and per-OS well-known roots (`Program Files\Java|Eclipse Adoptium|Zulu|Amazon Corretto|Microsoft`, `~/.jdks` on Windows; `/Library/Java/JavaVirtualMachines`, `~/Library/Java/JavaVirtualMachines`, homebrew `opt` on macOS; `/usr/lib/jvm`, `/usr/java`, `/opt/java`, `~/.sdkman/...` on Linux). Major version from the JVM `release` file (`JAVA_VERSION=`), else by running `java -version`. Dedup by canonical path, sort major desc.
2. **Download** (`ensure_runtime(dirs, client, major, component, progress)`): cache hit check on `java/<major>/` → try Mojang (os key + component, but only when the manifest entry's version name matches the requested major — e.g. `jre-legacy`=8, `java-runtime-delta`=21; mismatches like 1.16.5's stale `java-runtime-alpha` fall through) → download every `raw` file concurrently (16-way, SHA-1 + size verified, exec bits applied) into a staging dir, rename to `java/<major>` → else Adoptium single archive (SHA-256 verified, zip/tar.gz extracted, runtime root found, renamed).
3. **Selection** (`resolve_runtime(dirs, client, required: Option<&JavaVersion>, configured, progress)`): explicit `--java` wins; then system JVM with exact major; then managed exact-major (auto-download); then nearest system JVM; else error. `launch()` calls this before install so failures are fast.

## Non-Goals / Out of Scope

- LZMA downloads from Mojang (raw always available).
- `minecraft-java-exe` launcher stub (component never requested).
- Registry-based JVM detection on Windows; `os.version`-gated rules (see EPIC-u2ckm decision).
- Runtime *updates* / health checks on already-cached runtimes (cache is content-verified per file).

## Key Decisions

| Decision | Rationale |
|----------|-----------|
| Mojang per-file `raw` downloads primary, Adoptium archive fallback | Mirrors official launcher (epic note); Adoptium covers majors/OSes Mojang lacks (e.g. Java 8 for 1.16.5, arm64 jre-legacy) |
| Caching keyed by `java/<major>/`, verified via `release` file + per-file SHA-1 | Epic note `~/.mc-launcher/java/<major>/...`; atomic staging-rename prevents partial caches |
| Component accepted only if its manifest version major matches | `java-runtime-alpha` is Java 16 today but 1.16.5 needs Java 8 — version-consistency check forces the Adoptium fallback instead of launching with the wrong JVM |
| Exact-major system JVM → managed exact-major → nearest system JVM | KR3 "best available runtime, no user intervention"; avoids launching 1.8.9 on Java 25 when a download is possible |

## Phases

### Phase 1: Detection (TASK-28y9j)

- [ ] `core/src/java.rs`: `JavaRuntime`, `RuntimeSource`, `detect_system`, `runtime_major`/`read_release_major`/`probe_executable_major`, `major_from_version_string` (`1.8.0_442`→8, `8u202`→8, `21.0.7`→21)
- [ ] `managed_dir`/`managed_runtime`/`managed_runtimes` cache helpers
- [ ] `Error::JavaRuntime(String)`, improved `JavaNotFound` message

**Checkpoint**: `detect_system` finds JAVA_HOME/PATH/well-known JVMs and reports correct majors; unit tests green.

### Phase 2: Download (TASK-4pd8f)

- [ ] Mojang product + component manifest models; `download_mojang_runtime` (concurrent raw files, staging rename)
- [ ] Adoptium models + `download_adoptium_runtime` (SHA-256 verify, zip/tar.gz extraction)
- [ ] `download::sha256_file` `pub(crate)`; `sanitize_entry_path` → `pub(crate)`
- [ ] deps: `flate2` (rust_backend), `tar`, `sha2`

**Checkpoint**: fixture-server tests install a fake runtime through both paths; cache re-check is a no-op.

### Phase 3: Selection & wiring (TASK-9ik83)

- [ ] `resolve_runtime` policy (configured → system exact → managed exact → nearest)
- [ ] `launch()` resolves runtime first, passes executable into `build_command`
- [ ] CLI `java list` / `java install <major>`; `launch --java` help updated

**Checkpoint**: `cargo run -p mc-launcher -- java list` prints system JVMs; launch picks Java 21 for 1.21.4 without flags.

## Testing Strategy

- Unit: `major_from_version_string` / release-file parsing / `pick`-policy on fixture lists / staging-rename + cache-hit no-op.
- Fixture-server integration (pattern of `core/src/launch.rs:686-715`): serve product+component manifests and file bodies → `ensure_runtime` installs a verifiable runtime; Adoptium path via served assets JSON + tiny tar.gz.
- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`.

## Rollback Plan

Selection defaults to the old `resolve_java` (JAVA_HOME/PATH) behavior when no runtime can be resolved/downloaded; the `launch()` change is one call site — reverting to `options.java` passthrough restores previous behavior. `java` subcommand is additive.

## Pre-Approval Checklist

- [x] Ground Truth refs verified against current codebase
- [x] Cross-plan contracts are referenced, not redefined
- [x] No speculative code — all patterns derived from existing source
- [x] Plan is under ~200 lines

## References

<!-- CROSS-PLAN CONTRACTS: If this plan defines an interface consumed by other plans,
use a `## Cross-Plan Contract: [Name]` section as the canonical definition.
Other plans reference it: > **Contract source**: PLAN-xxxxx §Section Name -->

### Cross-Plan Contract: java module API

> **Contract source**: PLAN-r4vnn §Approach

`mc_launcher_core::java` exposes:
- `detect_system() -> Vec<JavaRuntime>` (sync, cheap)
- `ensure_runtime(&Directories, &reqwest::Client, major: u32, component: Option<&str>, Option<ProgressFn>) -> Result<JavaRuntime>` (async download+cache)
- `resolve_runtime(&Directories, &reqwest::Client, required: Option<&JavaVersion>, configured: Option<&Path>, Option<ProgressFn>) -> Result<JavaRuntime>` (async, may download)
- `component_for_major(u32) -> Option<&'static str>`
- `managed_runtimes(&Directories) -> Vec<JavaRuntime>`

Consumed by the CLI (`java` subcommand, launch flow) and later by the Tauri app (EPIC-f75qf) and loader installs (EPIC-s7arr).
