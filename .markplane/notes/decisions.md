# Decision Log

Lightweight decision log. Format: `## YYYY-MM-DD: Decision Title`

---

## 2026-08-10: Instance layout & config schema

- Instance folder name = random hex id (`in-<16 hex>`); display name lives in the config so renames never break paths.
- `instance.json` (pretty JSON) per instance: `{ id, name, version, loader?, game_dir, created_at, last_played_at? }` with RFC 3339 UTC timestamps (dependency-free `core::clock`).
- Per-instance `game/` dir; shared artifacts (client jar, libraries, assets) stay global under `downloads/`.
- Launcher-level `config.json` supports optional directory overrides (`instances_dir`, `downloads_dir`, `accounts_dir`, `java_dir`); relative paths resolve against the data root.
- Import/export uses ZIP (deflate); import rejects `..`, absolute, and drive-letter entries.

## 2026-08-10: Vanilla launch pipeline (EPIC-u2ckm)

- Downloads: `.part` files with HTTP range resume, SHA-1+size verification before rename; verified files are reused without a request (TASK-458cd).
- Asset layout: content-addressed store at `downloads/assets/objects/<h0h1>/<hash>`; virtual indexes materialize `assets/virtual/<id>/`, legacy (non-virtual) indexes materialize inside the game dir `assets/` with the index JSON copied there too (TASK-ra35r).
- Rule semantics match the official launcher: deny by default, last matching rule wins; `os.version` regexes unsupported (treated as non-matching); `natives` classifiers support `-arm64` selection and legacy `${arch}` placeholders; maven coordinates accept 4-part `group:artifact:version:classifier` names (TASK-wpwuz).
- Launch uses an **offline profile** (v3 UUID over `OfflinePlayer:<name>`, `legacy` user type) until Microsoft auth lands (TASK-ix345); system Java (`--java`, `JAVA_HOME`, PATH) until the Java runtime epic (EPIC-8g7y2). Game stdout/stderr streams to `<gameDir>/logs/launcher/<ts>.log` (TASK-y2n5u, TASK-qc438).

