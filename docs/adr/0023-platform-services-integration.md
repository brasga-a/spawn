# ADR-0023 — Platform Services Integration (Steamworks and Others)

**Status:** Proposed  
**Scope:** Integration with store/platform services: Steamworks first, others (e.g. Epic Online Services) later.  
**Related:** [ADR-0008](0008-third-party-license-policy.md) (license policy: royalty-free proprietary SDKs under isolation rules), [ADR-0016](0016-networking-and-replication.md) (authentication, connect tokens), [ADR-0018](0018-gamepad-input.md) (Steam Input), [ADR-0024](0024-packages-and-game-extensions.md) (entitlements for DLC), [ADR-0025](0025-mods-and-workshop.md) (Workshop).

## Context

Shipping on Steam brings services players expect: achievements, stats, leaderboards, cloud saves, rich presence, friends, lobbies and invites, the overlay, authentication tickets, DLC ownership, and the Workshop. The Steamworks SDK is **proprietary and royalty-free** (Valve's SDK agreement); its runtime library (`steam_api64.dll` / `libsteam_api.so`) must ship with the game. [ADR-0008](0008-third-party-license-policy.md) allows royalty-free proprietary SDKs under its isolation rules. Steam's store revenue share is a distribution fee, not an SDK royalty.

## Decision

1. **Isolation (per ADR-0008):** platform SDKs (Steamworks now; others by their own ADR) live **only** in dedicated, optional integration crates, behind Cargo features enabled by the game. They are never dependencies of engine crates, frameworks, or tools; dedicated servers link them only when they need them (e.g. ticket validation).
2. **Contract crate `spawn-services`** (`PLANNED`): Spawn-defined data types and a narrow interface for the services a game uses: identity, achievements, stats, leaderboards, cloud saves, rich presence, friends/lobbies/invites, entitlements (DLC ownership), authentication tickets, and user-generated content (Workshop). It contains no SDK code. Its surface grows only with features a game actually uses.
3. **Backend crates:** `spawn-services-steam` (`PLANNED`) implements the contract with Steamworks; a **null/offline backend** in `spawn-services` lets games, tests and CI run without any platform. Binding choice (an existing Rust binding crate or thin in-house FFI) is decided at implementation, checking how the SDK is obtained and redistributed.
4. **Engine independence:** engine crates never see Steam types or IDs; games map platform identities to Spawn account identifiers.
5. **Authentication:** dedicated servers validate platform authentication tickets (e.g. Steam session tickets) through the backend before issuing or accepting connect tokens ([ADR-0016](0016-networking-and-replication.md)). The game protocol stays Spawn's own.
6. **Steam networking relays (SDR)** may be evaluated later as a transport option under ADR-0016; not adopted now.
7. **Steam Input:** behavior with Spawn's native gamepad backends is specified in [ADR-0018](0018-gamepad-input.md).
8. **Offline and non-Steam builds** must work: every service call can fail or be unavailable, and games handle it (typed errors, [ADR-0009](0009-error-handling-and-diagnostics.md)).

## Alternatives

- **Steamworks calls directly in game or engine code:** fastest, but spreads a proprietary SDK everywhere and locks the engine to one store.
- **No platform services:** fully open, but excludes achievements, Workshop and Steam authentication that players expect.
- **A generic multi-platform service layer from day one:** speculative until a second platform is targeted; the contract starts with what the game needs.

## References

Steamworks documentation (Valve), Epic Online Services documentation (cross-platform services, later), Godot/Bevy community Steamworks integrations (design references).

## Consequences

- Games on Steam ship `steam_api` alongside the executable; builds for other stores do not.
- This ADR is the adopting record for Steamworks required by ADR-0008's isolation rules; the SDK terms are verified and recorded here at implementation.

## Risks

- **SDK license terms** restrict redistribution of SDK files (headers, libraries) in a public repository. Mitigation: verify the terms before committing anything; obtain the SDK through the documented process if redistribution is not allowed.
- **Contract shaped by Steam** makes other platforms awkward later. Mitigation: Spawn-defined types; the second backend reviews the contract.
- **Service outages** affecting gameplay. Mitigation: services are never required for local or offline play; servers degrade gracefully.

## Revisit triggers

A second store or platform is targeted; Steam adds or changes services the game depends on.

## Acceptance conditions

Owner review.
