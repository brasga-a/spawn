# Implementation Plan — Foundation

**Status:** PLANNED. Tasks are incremental; each step should keep the workspace buildable.

## Ordered increments

| Phase | Deliverable | Exit evidence |
| --- | --- | --- |
| 0. Decisions | Review and accept/adjust ADRs 0001–0003. Choose target smoke-test platform. | Accepted ADRs and recorded platform scope. |
| 1. Clock contract | Fixed-step timing policy in `spawn-core` with injected elapsed durations. | T01, T02 headless tests passing. |
| 2. Platform | Small native window/event pump with raw event mapping in `spawn-platform`. | Window visible, close works, T05 and T10 as applicable. |
| 3. Renderer | Surface/device setup, clear/present and resize/recovery policy in `spawn-render`. | Clear frame; T06, T07, manual resize/minimize. |
| 4. Input | Device transitions and tick-based input sampling in `spawn-input`. | T03, T04 pass without window. |
| 5. Composition | Root `spawn-engine` binary wires subsystems, handles errors and cleanup. | T08, T09 and full smoke run; no unrelated crates changed. |
| 6. Completion | Document tested platforms, collect evidence, update statuses. | [Verification exit gate](VERIFICATION.md) satisfied. |

These are feature increments, not necessarily one pull request each. Keep each change reviewable; do not add all backends, editor UI, ECS or gameplay to get a clear frame.

## Implementation guardrails

- Follow [root AGENTS.md](../../AGENTS.md) and each affected crate's local AGENTS.md.
- Proposed ADRs are review inputs; do not silently implement them before acceptance.
- Avoid speculative generic interfaces. Document the concrete boundary only after the first consumer exists.
- The default binary is an integration/demo harness, not a production game launcher or editor.
- Make errors visible and resource lifetimes testable before adding convenience abstractions.
- If an external crate is selected, pin and justify it according to workspace dependency rules; check current upstream documentation before coding.
- Keep changes unrelated to this milestone out of the PR.

## Deferred follow-up

The **next** milestone may render a triangle, then mesh/camera; subsequently ECS/world integration, gameplay and multiplayer. Those need their own specs and acceptance criteria and must not be implied by success of this foundation slice.
