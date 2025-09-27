# Presenter Agent Handbook

## Mission
Deliver a purpose-built worship presentation platform focused on lyrics, scripture, timers, and stage displays with reliability levels suitable for live services.

## Current Status (2025-09-27)
- Documentation-only repository; no application code has been scaffolded yet.
- Architecture evaluation in progress (Electron + web, native desktop, or hybrid render pipeline).
- All future work must include automated tests and align with these guardrails.

## Project Structure & Key Files
- Root contains documentation and project guardrails; application code forthcoming.
- `docs/` holds domain research and should be expanded with ADRs, UX flows, and data schemas.
- `docs/functional-needs.md` (@docs/functional-needs.md) captures functional requirements—keep synchronized with implemented features.
- Plan to add `apps/controller/`, `apps/stage-display/`, and `packages/core-domain/` directories when scaffolding code; mirror this file structure with subsystem-specific AGENTS.md files.

## North Star Architecture (2025)
- **Modular core:** Separate domain services for lyrics, scripture, scheduling/timers, and stage-display orchestration. Keep inter-service communication via explicit contracts (TypeScript interfaces, Protobuf, or JSON schemas) to ease multi-app clients.
- **Presentation engine:** Target GPU-accelerated rendering (WebGPU/Canvas via Electron or platform-native compositor) with 60fps baseline and deterministic clock sync.
- **Sync & resilience:** Support offline-first data with eventual consistency syncing to cloud storage (SQLite primary, replicating via Litestream or equivalent). All user actions should be event-sourced to enable audit trails and rollback.
- **Extensibility:** Provide plugin surface via sandboxed modules (e.g., WASM or isolated worker threads) for custom display logic without risking controller stability.

## Domain Pillars
- **Lyrics:** Version-controlled song arrangements, rapid set management, real-time key changes. Enforce metadata schema (title, authors, CCLI, arrangement). Ensure timed cues map cleanly to display cues.
- **Bible:** Support multiple translations with structured references. Cache approved translations locally; design API for verse range queries and responsive readings.
- **Timers:** Global clock plus per-segment timers with overrun warnings (visual + optional haptics). Provide deterministic countdowns tied to display refresh.
- **Stage Displays:** Configurable layouts for lyrics, timers, notes, and alerts. Must support multi-display routing and remote preview endpoints.

## Development Workflow (TDD-First)
- When evaluating new tools, libraries, or frameworks, perform current online research to confirm selections represent state-of-the-art 2025 technology, architecture, and design; document sources and rationale in the PR.
1. Capture requirements as executable specs (unit tests, component tests, or contract tests) before implementing features.
2. Maintain red-green-refactor loops; no production code without failing tests first.
3. When scaffolding a new stack, bootstrap the testing toolchain in the same commit (e.g., Vitest/Jest + Playwright + Pact).
4. Keep work in small, reviewable commits with descriptive messages.

## Testing & Quality Gates
- **Minimum bar:** Unit, integration, and end-to-end layers must exist before feature completeness is claimed.
- **Golden paths:** Add snapshot/visual regression tests for stage-display compositions.
- **Continuous validation:** Configure CI (GitHub Actions) with `lint`, `typecheck`, `test`, and `build` jobs; builds fail on warnings.
- **Release safety:** Require canary runs using fixture data that represent Sunday service flows.

## Environment & Tooling Expectations
- Use Node.js LTS (≥22) with pnpm for JavaScript/TypeScript workflows. For native modules, ensure cross-platform builds (Windows/macOS) via prebuild.
- Treat infrastructure as code: dev containers or Nix flakes to guarantee reproducibility.
- Document any new dependency in `docs/` and update this file with commands.
- Sample baseline once code exists:
  - `pnpm install`
  - `pnpm lint`
  - `pnpm test`
  - `pnpm build`
  Update these commands if a different stack is chosen.

## Coding Standards & Patterns
- Favor domain-driven design with immutable domain events and read-model projections for UI state.
- Adopt strongly typed APIs (TypeScript strict mode, Zod/io-ts validation, or Rust/Kotlin for native services).
- Enforce accessibility (WCAG 2.2 AA) for operator/stage UIs; include automated accessibility audits (axe, pa11y).
- Keep UI components declarative; prefer reactive state management (React Server Components + Vite/Electron, or Solid/Qt/QML equivalents) with clear separation from rendering engine.

## Security & Data Protection
- Encrypt local caches at rest (SQLCipher or platform keychain). Secure all sync endpoints with mTLS and signed JWTs.
- Handle scripture/licensing data per provider agreements; log access for compliance.
- Store credentials/secrets via OS keychain integrations; never commit secrets.

## Observability & Operations
- Embed structured logging (JSON) with trace correlation IDs. Prepare to integrate OpenTelemetry for metrics/traces once services exist.
- Capture session logs and operator actions for post-service analysis; redact personal data by default.


## Branching & Review Protocol
- Work exclusively on feature branches named after the task (e.g., `feature/lyrics-import`). Never commit directly to `main`—this is a strict rule.
- Create a GitHub pull request immediately after branching so web-based reviews can proceed continuously; update the PR description as context evolves.
- Commit and push frequently to keep the PR conversation active and ensure reviewers see incremental progress.
- All merges to `main` must occur via GitHub PR after an explicit user review and approval; fast-forward or direct merges from local environment are forbidden.

## Final Merge Preparation Checklist
- All relevant files are committed with no pending changes or untracked artifacts.
- The branch is pushed to the remote and the associated PR contains the latest commits and status updates.
- Automated test coverage fully exercises new functionality, prioritizing functional/end-user behavior; include integration and end-to-end tests as necessary.
- Every Markdown document affected by the work is updated for consistency, especially architecture-focused docs and functionality guides.
- Confirm the implementation reflects state-of-the-art 2025 design decisions and capture those insights in documentation and PR notes.

## Collaboration & Decision Log
- Record architectural decisions in `docs/adr/` using numbered ADRs; update this file with references (`@docs/adr/0001-initial-stack.md`).
- Surface open questions in pull requests and ensure closure before merging feature branches.

## Immediate Next Actions
- Evaluate rendering stack options and create ADR for selection.
- Scaffold initial application with concurrent test suite setup (TDD-compliant).
- Define canonical fixture dataset (songs, scriptures, timers) for regression tests.
