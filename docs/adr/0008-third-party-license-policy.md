# ADR-0008 — Third-Party License and Dependency Policy

**Status:** Accepted  
**Scope:** All dependencies (runtime, build, tools) and all external code used as reference.  
**Related:** [ADR-0004](0004-in-house-graphics-api.md), [ADR-0005](0005-physics-strategy.md), [ADR-0006](0006-in-house-audio.md) (per-ADR license notes this policy consolidates), [ADR-0007](0007-toolchain-lints-and-ci.md) (CI enforcement).

## Context

Spawn is licensed under **MIT** (`LICENSE`) and its repository is public. Several ADRs rely on external references with different licenses (Jolt, Unreal, Quake, Steam Audio). Games published on a store need that store's proprietary SDK (e.g. Steamworks for Steam).

**Owner rule:** Spawn never pays royalties. Proprietary code is **not** forbidden in itself: royalty-free proprietary SDKs (such as Steamworks for games published on Steam) are acceptable. Copyleft code inside Spawn would force license changes on the whole engine and on games built with it.

**Revision:** an earlier version of this ADR forbade all proprietary code in shipped products. The owner revised it: the deciding criterion is payment (royalties), not whether the code is open.

## Decision

### Dependencies

| Category | Licenses / terms | Rule |
| --- | --- | --- |
| **Allowed** | MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, zlib, ISC, BSL-1.0, CC0-1.0, Unicode-3.0 / Unicode-DFS-2016, and dual licenses including one of these | May be used anywhere after the root dependency justification |
| **Review required** | MPL-2.0 and other file-level copyleft | Owner approval per crate, recorded in this ADR |
| **Allowed with conditions** | **Proprietary, royalty-free** (e.g. Steamworks SDK, platform/vendor SDKs, user-mode anti-cheat SDKs offered without royalties) | Allowed under the isolation rules below, each adopted by its own ADR |
| **Forbidden** | **Royalties**, revenue share paid to the vendor, license fees tied to revenue, budget or units sold (e.g. FMOD/Wwise commercial tiers, Havok) | Never added. Other one-time paid licenses require explicit owner approval |
| **Forbidden in anything shipped** (engine, games, servers) | Strong copyleft: GPL, LGPL, AGPL, SSPL; non-commercial licenses | Never added |
| **Development-only tools** (compilers, profilers not linked or redistributed) | Any license that allows use, including proprietary freeware (e.g. vendor GPU profilers) | Must never be linked into Spawn binaries unless they also satisfy a shipped category |

A store's distribution fee (e.g. Steam's revenue share for selling on Steam) is the cost of the store, not an SDK royalty, and does not violate this policy.

### Isolation rules for proprietary code

Spawn's engine stays MIT and buildable by anyone without proprietary components:

1. **Engine crates, frameworks and tools never require proprietary code.** Proprietary SDKs live in dedicated, optional integration crates (e.g. `spawn-services-steam`, [ADR-0023](0023-platform-services-integration.md)) or in game projects, behind Cargo features enabled by the game.
2. **The public repository contains only what the SDK's terms allow.** SDK headers, libraries or binaries are committed only if their license permits redistribution in a public repository; otherwise they are obtained through the vendor's documented process.
3. **No proprietary code is copied into MIT-licensed files.** Integration crates contain Spawn's own code calling the SDK.
4. **Each proprietary dependency** is adopted by its own ADR, recording its terms, redistribution rules and what it is used for.

Every dependency must also satisfy: maintained (recent releases or stable and complete), no unnecessary default features, and a stated reason (root [Workspace](../../AGENTS.md#workspace)).

### Using external code as reference

| Use | Rule |
| --- | --- |
| **Study** (reading code, docs, talks, papers) | Always allowed |
| **Port or close adaptation** of code under an allowed license | Allowed; keep the original copyright and license notice in the affected files and list it in `THIRD_PARTY_NOTICES.md` |
| **Copying or porting** code under GPL, the Unreal EULA, Source SDK, or any license that does not grant that right | **Forbidden**; study only. Using a proprietary SDK through its API is not copying |
| **Data** (HRTF datasets, controller mapping databases, fonts) | License checked per dataset; notice recorded |

AI-generated code is subject to the same rules: an agent must not reproduce code from forbidden sources, and code closely derived from an allowed source must carry its notice.

### Enforcement

- `cargo-deny` in CI checks licenses, duplicate versions, and security advisories (RustSec).
- `THIRD_PARTY_NOTICES.md` is created with the first dependency or ported code and kept current.

## Alternatives

- **Forbid all proprietary code** (earlier version of this ADR): keeps everything open, but excludes store SDKs such as Steamworks; revised by the owner.
- **Allow LGPL via dynamic linking:** technically possible, but adds legal and packaging complexity for games; rejected.
- **No automated enforcement:** relies on memory; rejected.

## Consequences

- Choices such as Havok, FMOD and Wwise remain excluded because of royalties or revenue-tied fees.
- Steamworks and other royalty-free platform SDKs are possible through isolated integration crates.
- Ports from Jolt (MIT) or SDL (zlib) are possible with notices.

## Risks

- **Transitive dependencies** may introduce forbidden licenses. Mitigation: `cargo-deny` checks the full tree.
- **License changes upstream** in new versions. Mitigation: pinned versions; `cargo-deny` on every update.
- **Proprietary SDK terms changing** (e.g. introducing fees). Mitigation: terms recorded in the adopting ADR; reviewed on every SDK update.
- **Accidental reproduction** of forbidden code by AI agents. Mitigation: agents follow this ADR; reviews check for unexplained code resembling forbidden sources.

## Revisit triggers

A required capability exists only under a review-required or paid license; console platforms (proprietary SDKs under NDA, usually royalty-free but with certification requirements) become a target.

## Acceptance conditions

Accepted by the owner, then revised by the owner (proprietary code allowed when royalty-free, under isolation rules). Applying it (configuration files, manifests, code) is an implementation task.
