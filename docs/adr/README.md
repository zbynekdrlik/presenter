# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) documenting significant technical decisions.

## Lifecycle States

- **Proposed**: Under discussion, not yet implemented
- **Accepted**: Approved and implemented
- **Deprecated**: Superseded by newer decision
- **Rejected**: Considered but not adopted

## ADR Index

| Number | Title | Status | Date |
|--------|-------|--------|------|
| [0001](0001-architecture-stack.md) | Backend Architecture Stack | Accepted | 2025-09-27 |
| [0002](0002-bible-sources.md) | Bible Source Pipeline | Accepted | 2025-09-27 |
| [0003](0003-dockerized-demos.md) | Dockerized Demo Environments | Accepted | 2025-10-01 |
| [0004](0004-resolume-settings-and-integration.md) | Resolume Settings & Integration | Proposed | 2025-09-30 |
| [0005](0005-stage-heartbeat.md) | Stage Display Heartbeat | Accepted | 2025-10-01 |

## Creating New ADRs

1. Copy `template.md` to `NNNN-<slug>.md` (use next available number)
2. Fill in all sections
3. Set status to "Proposed"
4. Link from PR description
5. Update this index
6. Change status to "Accepted" when implemented

## Template

See [template.md](template.md) for the standard ADR format.
