# Decision Log

Lightweight decision log. Format: `## YYYY-MM-DD: Decision Title`

---

## 2026-08-10: Instance layout & config schema

- Instance folder name = random hex id (`in-<16 hex>`); display name lives in the config so renames never break paths.
- `instance.json` (pretty JSON) per instance: `{ id, name, version, loader?, game_dir, created_at, last_played_at? }` with RFC 3339 UTC timestamps (dependency-free `core::clock`).
- Per-instance `game/` dir; shared artifacts (client jar, libraries, assets) stay global under `downloads/`.
- Launcher-level `config.json` supports optional directory overrides (`instances_dir`, `downloads_dir`, `accounts_dir`, `java_dir`); relative paths resolve against the data root.
- Import/export uses ZIP (deflate); import rejects `..`, absolute, and drive-letter entries.

