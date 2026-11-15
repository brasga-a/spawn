# ADR-0019 — Anti-Cheat and Client Trust

**Status:** Accepted  
**Scope:** Engine-level rules for trusting client data; the FPS framework builds game-specific checks on top.  
**Related:** [ADR-0016](0016-networking-and-replication.md) (networking), [ADR-0005](0005-physics-strategy.md) (lag compensation).

## Context

A competitive FPS attracts cheats: aimbots, wallhacks, speed hacks, packet manipulation. Kernel-level anti-cheat systems are invasive, run with full system privileges, and do not run on Linux, which is a primary target. Server authority already removes many cheat classes.

## Decision

1. **No kernel-level anti-cheat.** Defense relies on server authority, validation and detection. A proprietary **user-mode** anti-cheat is possible only through evaluation track A and its own ADR.
2. **The client is untrusted by construction:** clients send inputs only; the server computes movement, hits, damage and outcomes (root [Multiplayer](../../AGENTS.md#multiplayer-foundational)).
3. **Input validation on the server:** rate limits per tick, ranges (e.g. view-angle change per tick, movement input magnitude), timestamp plausibility (bounded by the lag-compensation window of [ADR-0016](0016-networking-and-replication.md)), and rejection of malformed or out-of-order commands.
4. **Information minimization against wallhacks:** the server does not replicate information a client cannot plausibly perceive. Interest management exposes hooks for visibility/audibility-based relevancy; the FPS framework implements potentially-visible checks. Exact positions of distant hidden enemies are not sent.
5. **Server-side replay recording** of inputs and authoritative events, enabling review and statistical detection (e.g. aim analysis) as later work.
6. **Encrypted, authenticated traffic** ([ADR-0016](0016-networking-and-replication.md)) prevents trivial packet injection and replay.
7. **Humans decide penalties.** Automated systems (validation, AI models, integrity checks) only reject invalid inputs in the current match and produce flags; any penalty against a player is decided by a human reviewer.
8. **Disclosure:** the game's Terms of Service and Privacy Policy describe the anti-cheat data collected, how it is analyzed, the human review process, and possible penalties.
9. **No security through obscurity** as a primary defense; client-side checks may be added later as a deterrent only, never as a source of truth.

### Evaluation tracks (planned experiments, not adoptions)

Each track is run as a measured experiment. Adopting the result requires its own ADR. None may introduce kernel-level code.

**A. Proprietary user-mode anti-cheat (e.g. Easy Anti-Cheat, BattlEye in user mode).**
- Goal: measure what a commercial user-mode solution adds beyond server-side defenses.
- Criteria: Linux/Proton and Steam Deck compatibility, cost, performance impact on the reference hardware, privacy (what data it collects), integration effort, and detection effectiveness against public cheats in a test environment.
- Adoption requires its own ADR and must be royalty-free under [ADR-0008](0008-third-party-license-policy.md) (proprietary code is allowed under its isolation rules). Integration would live at the game level behind a feature, never in engine-core crates.

**B. Small AI models for behavioral detection (server-side).**
- Inputs: server-recorded replays and telemetry (decision 5): view-angle deltas, snap angles, time-to-target, reaction times, tracking smoothness, hit distributions.
- Models: small models (e.g. gradient-boosted trees or small neural networks) trained offline; inference on the server or in offline batch jobs, never on the client.
- Output: **analysis and flags only. AI never punishes.** No automated system (AI or rule-based) applies any penalty: bans, kicks, matchmaking restrictions, account limits or hidden measures. A flagged player is evaluated by a human, and every penalty is a human decision, recorded with the evidence reviewed.
- Requirements: false-positive rate measured on legitimate players before any use; documented data retention; retraining as cheats adapt.
- Reference: Valve's VACnet ("Robocalypse Now: Using Deep Learning to Combat Cheating in CS:GO", GDC 2018).

**C. Binary and asset integrity analysis.**
- **Client integrity:** signed release builds; the server can challenge clients for hashes of critical assets (e.g. models and textures, to detect transparent-wall modifications) and of executable sections.
- **Cheat analysis:** reverse engineering of public cheats in an isolated lab environment to understand their techniques and derive server-side detections or user-mode signatures.
- **Deterrents:** obfuscation and anti-debugging may be evaluated as deterrents only (decision 9).
- Limits: any client-side scanning is user mode, restricted to the game process and its loaded modules, never personal files, and is documented publicly.

## Alternatives

- **Kernel anti-cheat (EAC, BattlEye, Vanguard):** strong against some cheats; proprietary, invasive, does not run on Linux, security risk of ring-0 drivers (vulnerable anti-cheat drivers have been abused by malware), bypassed by DMA hardware and external computer-vision cheats. Rejected.
- **Client-side user-mode checks only:** easily bypassed; at most a later deterrent.

## Consequences

- Linux players are not excluded by anti-cheat (a common problem with kernel solutions).
- Some cheats (aim assistance software reading the screen) cannot be prevented, only detected statistically.

## Risks

- **AI false positives** flag legitimate high-skill players. Mitigation: AI only flags; a human evaluates every flag and decides any penalty; false-positive rate measured and tracked.
- **Telemetry privacy.** Mitigation: collect only gameplay data needed for detection; documented retention; anti-cheat data collection, analysis, human review and possible penalties disclosed in the game's **Terms of Service and Privacy Policy**. Terms of Service alone may not satisfy data-protection law (e.g. LGPD in Brazil, GDPR in the EU: legal basis, data minimization, access and deletion rights); a legal review is required before public release.
- **Integrity challenges bypassed** by modified clients that answer with expected hashes. Mitigation: integrity checks are **signals, never proof**; a failed or suspicious check only adds a flag for human review, combined with server-side behavior analysis.
- **Aimbots remain possible** because aiming input is legitimately client-driven. Mitigation: statistical detection from replays; reporting tools; this is accepted as a residual risk.
- **Visibility culling cost** on the server at large player counts. Mitigation: coarse relevancy first, measured.
- **False positives** in validation causing legitimate players to be corrected. Mitigation: tolerances tuned by play-testing; logging of rejections.

## Revisit triggers

Cheating measurably harms the game after release; a platform requires a specific anti-cheat.

## Acceptance conditions

Accepted by the owner. Applying it is an implementation task.
