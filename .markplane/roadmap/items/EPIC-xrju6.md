---
id: EPIC-xrju6
title: Microsoft account authentication
status: now
priority: critical
started: 2026-08-09
target: 2026-09-30
related: []
tags: []
created: 2026-08-09
updated: 2026-08-09
---

# Microsoft account authentication

## Objective

Implement full Microsoft account sign-in using the device code flow: obtain tokens, store them securely with refresh support, and use them for authenticated Mojang API calls (profile UUID, skins). Required for launching and later for Modrinth/CurseForge features that need auth.

## Key Results

- [ ] KR1: `mc-launcher login` completes device-code sign-in end to end
- [ ] KR2: Tokens persist across restarts and auto-refresh without re-login
- [ ] KR3: Profile UUID + name fetched and attached to launches

## Notes

- Flow: `https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode` → user enters code at `microsoft.com/link` → poll for token.
- Token storage must be encrypted at rest (keyring on macOS, DPAPI on Windows, libsecret on Linux).
- Azure app registration needed for client_id — decide whether to host a shared public client or document self-hosting.
