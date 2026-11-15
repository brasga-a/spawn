# ADR-0025 — Mods and Workshop

**Status:** Proposed  
**Scope:** User-made mods: what they may do, how they are distributed (Steam Workshop and local), synchronized in multiplayer, and moderated.  
**Related:** [ADR-0024](0024-packages-and-game-extensions.md) (package format, trust levels), [ADR-0017](0017-luau-scripting-runtime.md) (sandbox), [ADR-0023](0023-platform-services-integration.md) (Workshop backend), [ADR-0019](0019-anti-cheat-and-client-trust.md) (integrity, Terms of Service).

## Context

Modding extends a game's life and is a stated goal of the project. Mods are third-party, untrusted content that can arrive through the Steam Workshop, local folders, or a server requiring them. They must never compromise player machines, servers, or competitive integrity.

## Decision

1. **Mods are packages of kind `mod`** ([ADR-0024](0024-packages-and-game-extensions.md)): data, assets and sandboxed Luau only. **No native code**, no access to files, network or OS beyond the allowlisted script API.
2. **Validation on load:** manifest and data validated with size, count and nesting limits; assets checked against format rules; scripts run under the limits of [ADR-0017](0017-luau-scripting-runtime.md). A failing mod is disabled with a clear error and never crashes the game.
3. **Scopes:**
   - `client` mods: presentation only (UI, HUD skins, sounds, visual effects); cannot affect gameplay or server state.
   - `server` / `shared` mods: gameplay rules, maps, modes; active only on servers that enable them; clients joining must match exactly (hash-verified).
4. **Distribution:**
   - **Steam Workshop** through the `spawn-services` UGC interface ([ADR-0023](0023-platform-services-integration.md)): upload, subscribe, update.
   - **Local folder** for development and players outside Steam.
   - **Server-provided download:** a server may offer the packages it requires; clients verify content hashes and size limits before mounting. Downloads never execute anything.
5. **Competitive integrity:** official ranked/competitive modes run only with `base`/`official` packages and an allowlist of approved `client` mods; integrity checks ([ADR-0019](0019-anti-cheat-and-client-trust.md)) account for approved mods.
6. **Tooling:** `spawn-cli` provides commands to create a mod skeleton, validate it, cook its assets, and package it for upload. The editor may add a visual workflow later.
7. **Moderation and legal:** the Terms of Service cover user-generated content (ownership, permitted content, takedown process for infringing or harmful content). Workshop moderation follows Steam's tools; server-provided mods are the server operator's responsibility.

## Alternatives

- **mod.io or another cross-store mod service:** reaches non-Steam players; third-party service and SDK to evaluate under ADR-0008 and ADR-0023 if a second store is targeted.
- **Native-code mods (DLLs):** maximum power, but arbitrary code execution on players' machines; rejected.
- **Unrestricted Lua (no sandbox):** common in older games, a frequent source of malware in mods; rejected.

## References

Steam Workshop documentation, Factorio and Garry's Mod (server-driven mod sync), Roblox (sandboxed Luau user content), Minecraft and Skyrim modding history (security and load-order lessons).

## Consequences

- The game's script and data APIs become a public modding contract, versioned and documented ([ADR-0017](0017-luau-scripting-runtime.md)).
- Mods built for an API version keep working until a breaking API version; players are told which mods are incompatible.

## Risks

- **Malicious mods** exploiting sandbox bugs. Mitigation: sandbox-escape tests, allowlist review per API addition, quick disable mechanism.
- **Harmful or infringing content.** Mitigation: Terms of Service, Workshop moderation, takedown process.
- **Server-provided downloads abused** (huge files, malformed archives). Mitigation: size limits, hash verification, archive validation before mounting.

## Revisit triggers

Mod authors need capabilities the sandbox cannot provide; a second store requires cross-store mod distribution.

## Acceptance conditions

Owner review.
