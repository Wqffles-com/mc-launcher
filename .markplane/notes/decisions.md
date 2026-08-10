# Decision Log

Lightweight decision log. Format: `## YYYY-MM-DD: Decision Title`

---

## 2026-08-10: Microsoft auth (EPIC-xrju6)

- Client id: ship the well-known public Microsoft client `00000000402B5328` (used by community launchers for the Minecraft device flow) as `DEFAULT_CLIENT_ID`; `MC_LAUNCHER_CLIENT_ID` env overrides it for self-hosted Azure registrations. A shared hosted client is deferred until release.
- Scope: `XboxLive.signin offline_access` (device + refresh flows); Microsoft refresh tokens rotate, so every refresh persists the new pair.
- Token storage: per-account JSON at `accounts/<uuid>.json` holds metadata + short-lived Minecraft access token (plaintext — it expires in ~24 h); the long-lived refresh token goes to the OS keyring via the `keyring` crate (`windows-native`/`apple-native`/`sync-secret-service`). If the keyring is unavailable (e.g. headless Linux), the refresh token falls back into the JSON file with `"token_storage": "file"` so users can see it is unprotected.
- Player for launches: `Player::microsoft` uses the profile UUID (dash-less), real name, Bearer access token and `user_type: "msa"`; launch auto-refreshes expired tokens and falls back to the offline v3-UUID profile only when no account exists.
- Multi-account: default = most recently used (`last_used_at`); `account use` switches, `launch --account <uuid|name>` overrides.
- Token expiry tracked via `clock::rfc3339_utc` timestamps; refresh triggers when the Minecraft token expires.
- Keyring fallback policy: only `login` may move a keyring-stored refresh token into the plaintext file (announced); `touch`/`refresh` never silently downgrade — a keyring failure for a keyring-stored account is an error, and accounts already in `file` mode stay in `file` mode. Refresh also refetches the Mojang profile so player name changes propagate without re-login.

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

## 2026-08-10: Java runtime management (EPIC-8g7y2)

- Sources: Mojang runtime manifests primary (per-OS component manifests with per-file `raw` downloads — no LZMA), Adoptium single-archive fallback; overridable via `MC_LAUNCHER_JAVA_MANIFEST_URL` / `MC_LAUNCHER_ADOPTIUM_URL` for mirrors. Cache at `java/<major>/`, atomic staging rename, per-file SHA-1 (Mojang) / archive SHA-256 (Adoptium) verification.
- Component acceptance rule: a Mojang component is only used when its current manifest version's major matches the requested one — `java-runtime-alpha` is Java 16 today, so 1.16.5-style Java 8 requests fall back to Adoptium instead of launching with the wrong JVM (verified against the live manifest 2026-08-10).
- Selection order (TASK-9ik83): explicit `--java` → system JVM of exact required major (version JSON `javaVersion`, default 8 for pre-1.13) → auto-downloaded managed runtime of that major → nearest system JVM (smallest ≥ major, else newest) → error suggesting `mc-launcher java install <major>`.
- Detection (TASK-28y9j): JAVA_HOME, PATH, per-OS roots (Program Files\Java|Eclipse Adoptium|Zulu|Amazon Corretto|Microsoft + `~/.jdks`; macOS JVMs dirs + homebrew opt; `/usr/lib/jvm`, `/usr/java`, `/opt/java`, sdkman); major from the JVM `release` file, `java -version` fallback; dedup by canonical path.
- Fixed alongside: `download::partial_path` appended `.part` with `with_extension`, so `bin/java.dll` and `bin/java.exe` shared one `.part` file and corrupted each other during concurrent downloads — the suffix is now appended to the file name.

