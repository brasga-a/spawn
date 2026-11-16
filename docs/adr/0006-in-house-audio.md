# ADR-0006 — In-House Audio: Output, Mixer and Voice Management

**Status:** Accepted.  
**Scope:** `spawn-audio`. Client-only: dedicated servers do not link it.  
**Related:** [ADR-0001](0001-in-house-platform-layer.md) (binding policy, supported platforms), [ADR-0003](0003-fixed-step-loop-and-threading.md) (audio thread), [ADR-0005](0005-physics-strategy.md) (occlusion queries).

## Context

In an FPS, audio is gameplay: the direction of footsteps and gunshots decides engagements, headphone spatialization (HRTF) is a competitive advantage, and a large battle produces hundreds of simultaneous sounds that must stay intelligible. Large-scale shooters are known for dynamic "HDR audio" mixing, where loud sounds temporarily mask quieter ones. Gunshot feedback needs low output latency.

The owner requires **no royalties** ([ADR-0008](0008-third-party-license-policy.md)).

## Decision

1. **Output backends, in-house:**
   - Windows: WASAPI through `windows-sys` (binding policy of [ADR-0001](0001-in-house-platform-layer.md)).
   - Linux: **PipeWire only**, through thin bindings. No PulseAudio or ALSA fallback; systems older than the owner's reference hardware and software are not targeted.
   - A null backend for headless tests.
2. **Mixer, buses, effects, and voice management, in-house:** priorities, voice limits, virtualization of inaudible voices, and HDR-style dynamic mixing.
3. **Decoding, integrated:** Opus through `opus` (libopus, BSD-3) and Vorbis through `lewton` (pure Rust), approved in [DEPENDENCIES](../DEPENDENCIES.md); `symphonia` was rejected by the owner (MPL-2.0). Writing codecs is out of scope.
4. **No middleware:** FMOD and Wwise are excluded (license fees tied to revenue or budget in their commercial tiers). Sound events are declarative data (root [Data-Driven Design](../../AGENTS.md#data-driven-design-and-scripting-security)); a visual authoring tool is future `spawn-editor` work.
5. **Occlusion boundary:** `spawn-audio` never depends on `spawn-physics`. Occlusion and obstruction values are computed by higher layers (e.g. the FPS framework with physics raycasts) and passed to `spawn-audio` per voice.

## Spatialization: hybrid (owner-confirmed)

6. **In-house first, Steam Audio where in-house is expensive.**
   - **In-house:** HRTF for direct sound, distance attenuation, simple occlusion/obstruction, and their integration with the mixer and HDR mixing. Steam Audio is the design reference.
   - **Steam Audio** (Valve, Apache-2.0, no royalties) may be integrated later **only** for features whose in-house cost is high: sound propagation and pathing (around corners, through openings), reflections, and physically based reverb. Each integration needs its own ADR naming the feature, the measured need, and how scene geometry is supplied to it.
   - Steam Audio, if integrated, is DSP inside the in-house mixer; it never replaces the mixer, voice management, or output backends.

Trade-offs considered:

| Option | Pros | Cons |
| --- | --- | --- |
| In-house HRTF | Full control; pure Rust; tuned for competitive clarity; direct integration with the mixer and HDR mixing | Needs measured HRTF datasets (SOFA; license per dataset); interpolation and per-source convolution to implement and optimize; subjective quality needs listening tests; propagation and reflections are a separate large effort |
| Steam Audio | Mature HRTF, occlusion, transmission, reflections, reverb and pathing; custom SOFA HRTFs; free | C library (bindings, native build); less control over internals; roadmap decided by Valve; needs its own copy of scene geometry |

## References

| Reference | Use | License note |
| --- | --- | --- |
| Steam Audio | HRTF, occlusion, propagation design | Apache-2.0 |
| `kira`, `rodio`, `fundsp`, `oddio` | Rust mixer, scheduling and DSP designs | MIT / Apache-2.0 (check each) |
| FMOD and Wwise documentation | Concepts: events, buses, parameters, voice management | Proprietary: documentation only |
| *Game Audio Programming* book series (ed. Guy Somberg) | Engine audio architecture, voice management, spatialization | Books |
| Public HRTF datasets (SOFA format) | Measured HRTFs | License per dataset |

## Alternatives

- **FMOD / Wwise:** best authoring tools, widely used; rejected for revenue-tied license fees.
- **`cpal` for output:** portable Rust output; rejected in favor of in-house WASAPI/PipeWire backends, which are small and consistent with [ADR-0001](0001-in-house-platform-layer.md).
- **`kira` as the mixer:** complete Rust game-audio library; rejected because the mixer and voice management are where the game's sound identity lives.

## Consequences

- The real-time audio thread (already required by `spawn-audio`) is Spawn's code end to end, except decoding.
- Sound designers have no authoring tool until `spawn-editor` provides one; events are edited as data files.
- Only PipeWire-based Linux systems are supported.
- A later Steam Audio integration would add a C dependency to `spawn-audio`, behind a Cargo feature so builds without it stay pure Rust.

## Risks

- **Real-time safety:** allocation, locks or blocking I/O on the audio thread cause audible glitches. Mitigation: lock-free, allocation-free audio thread with bounded queues; stress tests.
- **Latency vs. underruns:** smaller buffers lower latency but risk underruns on loaded systems. Mitigation: configurable buffer size, measured on the reference hardware.
- **PipeWire-only:** excludes systems without PipeWire. Accepted by the owner.
- **HRTF quality** of the in-house implementation depends on subjective tuning. Mitigation: listening tests with recorded positions; comparison against Steam Audio as reference.
- **Voice explosion** in large battles. Mitigation: hard voice limits, priorities and virtualization from the first mixer version.

## Acceptance conditions

Accepted by the owner. Any Steam Audio integration requires its own ADR.
