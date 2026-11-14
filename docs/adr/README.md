# Architecture Decision Records

ADRs capture **why** a technical choice was made. They complement, but do not override, [root AGENTS.md](../../AGENTS.md) or the [foundation specification](../foundation/SPEC.md).

## Status definitions

- **Proposed:** candidate evaluated; implementation is **not authorized** by the ADR alone.
- **Accepted:** project owner/team has chosen this approach and related documentation has been synchronized.
- **Superseded:** replaced by a linked newer ADR.
- **Rejected:** considered and declined with rationale.

Update an ADR's status, alternatives, consequences, and supersession links when a decision changes. Never retroactively label an unreviewed proposal Accepted.

## Index

| ADR | Status | Topic |
| --- | --- | --- |
| [ADR-0001](0001-window-render-backends.md) | Proposed | Windowing and graphics backends |
| [ADR-0002](0002-event-ownership.md) | Proposed | Ownership of raw events and semantic input |
| [ADR-0003](0003-loop-ownership.md) | Proposed | Timing, event-loop and scheduling ownership |

These proposals are inputs to Phase 0 of the [implementation plan](../foundation/PLAN.md). The root project's unresolved architectural questions take precedence until decisions are explicitly accepted.
