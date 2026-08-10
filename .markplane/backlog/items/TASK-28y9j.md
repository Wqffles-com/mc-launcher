---
id: TASK-28y9j
title: System Java detection & scanning
status: done
priority: high
type: feature
effort: medium
epic: EPIC-8g7y2
plan: PLAN-r4vnn
depends_on: []
blocks: [TASK-4pd8f]
related: []
assignee: null
tags: []
position: aG
created: 2026-08-09
updated: 2026-08-10
---

# System Java detection & scanning

## Description

Find every usable JVM on the machine so launch can pick the right one without asking the user. Candidates come from `JAVA_HOME`, `java` entries on PATH, and per-OS well-known installation roots (Windows `Program Files\Java|Eclipse Adoptium|Zulu|Amazon Corretto|Microsoft` + `~/.jdks`; macOS `/Library/Java/JavaVirtualMachines` + homebrew `opt`; Linux `/usr/lib/jvm`, `/usr/java`, `/opt/java`, sdkman). For each candidate determine the Java major version — from the JVM `release` file (`JAVA_VERSION=`) when present, else by running `java -version` — deduplicate by canonical path, and sort by major descending. Implements Phase 1 of PLAN-r4vnn; the manifest itself comes from TASK-4pd8f.

## Acceptance Criteria

- [ ] `detect_system()` finds JVMs from JAVA_HOME, PATH, and well-known roots on all 3 OSes (roots covered by cfg-gated code + unit tests over fixture layouts)
- [ ] Major version is read from the `release` file when available and falls back to probing `java -version` otherwise (`1.8.0_442` → 8, `8u202` → 8, `21.0.7` → 21)
- [ ] Duplicate homes (e.g. same JVM via PATH and a well-known root) appear once; results are sorted by major descending

## Notes

- `core/src/java.rs`, modeled on the version-string parsing in `core/src/version_json.rs` and the PATH walk in `core/src/launch.rs:resolve_java` (which this supersedes for selection).
- Probing spawns `java -version`; the `release` file path avoids process spawns for most installs.

## References

- PLAN-r4vnn Phase 1
test
