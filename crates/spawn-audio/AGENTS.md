# spawn-audio

Local rules for `spawn-audio` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Audio runtime: device output, playback, mixing, and spatial sound.

## Responsibilities

- In-house output backends: WASAPI on Windows, PipeWire on Linux, plus a null backend ([ADR-0006](../../docs/adr/0006-in-house-audio.md); PLANNED).
- In-house mixing graph, buses, effects, voice management (priorities, limits, virtualization) and HDR-style dynamic mixing (PLANNED).
- Sound instances/voices, buses, effects, listener state, and in-house 3D spatialization (HRTF, attenuation, simple occlusion). Steam Audio may later provide propagation/reflections as DSP inside the mixer, behind a Cargo feature, only through its own ADR ([ADR-0006](../../docs/adr/0006-in-house-audio.md)).
- Streaming vs. resident audio policies.

## Non-Responsibilities

- Decoding policy and asset identity (`spawn-assets`; formats' runtime use is coordinated, not duplicated), file access (`spawn-io`), gameplay triggers (frameworks/Luau client presentation scripts), network voice transport (`spawn-network`).

## Architectural Boundaries

- May depend on `spawn-math`, `spawn-assets`; must not depend on render, physics, scripting, or gameplay crates. Occlusion/obstruction values are supplied per voice by higher layers. No FMOD/Wwise or other royalty-bearing middleware. Optional: not linked by games without audio, nor by dedicated servers.

## Public API Guidelines

- Voices referenced by generational handles; play requests are commands, not callbacks into game code.

## Internal Design Principles

- Real-time audio thread is lock-free and allocation-free; communication via bounded queues.

## Performance and Safety

- Underruns avoided by design; untrusted audio data bounded and validated; voice limits enforced.

## Testing Requirements

- Headless mixer tests with a null output device; spatialization math tests; stress tests for voice limits.

## Governing Decisions

- **Accepted (binding):** [ADR-0006](../../docs/adr/0006-in-house-audio.md) (audio), [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md) (platform layer). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `windows-sys`, `pipewire`, `rustix`, `opus`, `lewton`, `rubato`, `realfft`, `rustfft`, `sofar`. Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No audio playback or mixer exists.

## Agent Instructions

- Do not implement gameplay audio rules (e.g. footstep logic).
- Keep a null backend so tests and servers run without hardware.
