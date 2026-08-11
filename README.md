# mc-launcher

A cross-platform Minecraft launcher CLI and desktop app — vanilla and modded, with Modrinth & CurseForge support.

## Status

Early development. The roadmap is managed with [Markplane](https://github.com/zerowand01/markplane) inside this repo — see [`.markplane/roadmap/INDEX.md`](.markplane/roadmap/INDEX.md) for epics in `now` / `next` / `later` phases, or run `markplane serve` for the web UI.

Progress so far: Cargo workspace scaffolded (`core` library + `cli` binary); Mojang version manifest fetch/cache and version-JSON parsing landed; **instances & profiles are done** — `mc-launcher instance create/list/clone/delete/set-version/set-loader/export/import` (per-instance `game/` dirs, `instance.json` config, ZIP import/export). **Minecraft version management is done** — `mc-launcher install <version>` downloads the client jar, libraries, natives and assets with progress, resume and SHA-1 verification; `mc-launcher launch <version|--instance>` resolves rules-based JVM/game arguments, spawns Java with an offline profile, streams game output to `logs/launcher/<ts>.log` and reports exit codes. **Microsoft account authentication is done** — `mc-launcher login` runs the device code flow (`microsoft.com/link`), exchanges tokens through Xbox Live/XSTS, fetches the Minecraft profile, and stores it in the OS credential store (Windows Credential Manager / macOS Keychain / Linux Secret Service) with automatic refresh when tokens expire; `mc-launcher account list|remove|use` manages multiple accounts and `mc-launcher launch --account <uuid|name>` launches as a signed-in player (offline profile remains the fallback). The Azure client id defaults to the well-known public Minecraft client; set `MC_LAUNCHER_CLIENT_ID` to use your own registration. **Java runtime management is done** — launch auto-selects the best runtime per version: a system JVM of the required major (`JAVA_HOME`, PATH, well-known install locations; detected via the JVM `release` file or `java -version`), or an auto-downloaded managed runtime cached under `~/.mc-launcher/java/<major>/` (Mojang's runtime manifests, verified per file, with Adoptium archives as fallback), so users never need to install Java; `mc-launcher java list` shows system + managed runtimes and `mc-launcher java install <major>` downloads one on demand; `--java` still overrides. **Fabric loader support is done** — `mc-launcher fabric list [game]` shows the game versions Fabric supports (or the loaders for one), `mc-launcher fabric install <game> [--loader <v>]` installs a full Fabric profile (fetched from `meta.fabricmc.net`, merged with the game version — loader libraries from `maven.fabricmc.net` included, loader-pinned libraries deduped over the game's — and all artifacts verified and cached); `instance set-loader fabric <v>` validates the loader version against the API, and `launch --instance` on a Fabric instance installs and launches the merged profile with Fabric's `KnotClient`. Try `cargo run -p mc-launcher -- install 1.21.4`, `cargo run -p mc-launcher -- login`, `cargo run -p mc-launcher -- launch --instance "My World"`, or `cargo run -p mc-launcher -- java list` (workspace builds with `cargo build --workspace`).

## Goals

- **Vanilla + modded**: install and launch any Minecraft release or snapshot
- **Mod loaders**: Fabric, Quilt, Forge, NeoForge
- **Mods & modpacks**: from Modrinth and CurseForge
- **Microsoft auth**: device-code sign-in with token refresh
- **Java**: auto-managed runtimes, no manual Java installs
- **Instances**: isolated, portable game profiles
- **Servers** (post-v1): run and manage vanilla/modded servers
- **Platforms**: Windows, macOS, Linux

## Architecture

- `core/` — Rust library: version manifest, downloads, auth, launch engine, loader install, mod APIs
- `cli/` — `mc-launcher` binary: power-user interface over `core`
- `app/` — Tauri v2 desktop app: React + TypeScript + shadcn/ui + Tailwind

## Roadmap

| Phase | Epics |
|-------|-------|
| Now | Project foundation |
| Done | Microsoft account authentication · Minecraft version management · Instances & profiles · Java runtime management · Fabric loader support |
| Next | Mod loaders · Desktop app |
| Later | Modrinth · CurseForge · Modpacks · Release & updates · Servers |

See the full [roadmap](.markplane/roadmap/INDEX.md) and [backlog](.markplane/backlog/INDEX.md).

## License

MIT — see [LICENSE](LICENSE).
