# mc-launcher

A cross-platform Minecraft launcher CLI and desktop app — vanilla and modded, with Modrinth & CurseForge support.

## Status

Early development. The roadmap is managed with [Markplane](https://github.com/zerowand01/markplane) inside this repo — see [`.markplane/roadmap/INDEX.md`](.markplane/roadmap/INDEX.md) for epics in `now` / `next` / `later` phases, or run `markplane serve` for the web UI.

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
| Now | Minecraft version management · Microsoft auth · Project foundation |
| Next | Instances & profiles · Mod loaders · Java runtime management · Desktop app |
| Later | Modrinth · CurseForge · Modpacks · Release & updates · Servers |

See the full [roadmap](.markplane/roadmap/INDEX.md) and [backlog](.markplane/backlog/INDEX.md).

## License

MIT — see [LICENSE](LICENSE).
