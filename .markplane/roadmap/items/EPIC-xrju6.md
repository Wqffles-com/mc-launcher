---
id: EPIC-xrju6
title: Microsoft account authentication
status: done
priority: critical
started: 2026-08-09
target: 2026-09-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-10
---

# Microsoft account authentication

## Objective

Implement full Microsoft account sign-in using the device code flow: obtain tokens, store them securely with refresh support, and use them for authenticated Mojang API calls (profile UUID, skins). Required for launching and later for Modrinth/CurseForge features that need auth.

## Key Results

- [x] KR1: `mc-launcher login` completes device-code sign-in end to end
- [x] KR2: Tokens persist across restarts and auto-refresh without re-login
- [x] KR3: Profile UUID + name fetched and attached to launches

## Notes

- Flow: `https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode` → user enters code at `microsoft.com/link` → poll for token.
- Token storage must be encrypted at rest (keyring on macOS, DPAPI on Windows, libsecret on Linux).
- Azure app registration needed for client_id — decide whether to host a shared public client or document self-hosting.

## Done (2026-08-10)

- `core/src/auth.rs`: device code request, polling (pending/slow_down/declined/expired), XBL → XSTS → Minecraft token chain, profile fetch, refresh chain; `_at` endpoint variants for mock testing. Client id from `MC_LAUNCHER_CLIENT_ID` env, else the well-known public client `00000000402B5328` (see decisions log).
- `core/src/accounts.rs`: multi-account store in `accounts/<uuid>.json`; refresh tokens in the OS keyring (Windows Credential Manager / macOS Keychain / Linux Secret Service) with plain-file fallback flagged `"token_storage": "file"`; auto-refresh of expired Minecraft tokens; `AccountManager` list/get/remove/touch/default.
- `launch::Player::microsoft` attaches real UUID, name and token to game args (`user_type: "msa"`); offline profile remains the fallback.
- CLI: `mc-launcher login`, `mc-launcher account list|remove|use`, `mc-launcher launch --account <uuid|name>` (defaults to most recently used account; offline fallback).
- 20 new tests: scripted mock-server coverage of the whole flow + account store roundtrips.
