# ADR 0001: Backend Architecture Stack Selection

## Status
Accepted – 27 September 2025

## Context
Presenter must deliver sub-100 ms control latency, multi-channel networking (Web UI, WebSockets, OSC, MIDI), and run reliably on an Intel N100 Linux controller with limited headroom. We need a stack that offers:

- High-performance async I/O for WebSockets, HTTP, and custom protocols.
- Strong typing across the domain model to keep complex slide/Bible logic maintainable.
- Excellent tooling for test-first development and future static binaries for deployment.
- Momentum in 2025 with active maintenance and ecosystem support for the integrations we require (Resolume, Companion, MIDI/Bome).

## Decision
Adopt a Rust-first monolithic backend built on `axum` 0.8 + `tokio` 1.40, with domain logic isolated in a standalone crate. We'll layer SeaORM + SQLx over SQLite for persistence and plan for a Leptos 0.7 front-end shell rendered from the same binary.

Key elements:

- `presenter-core` crate encapsulates domain types (libraries, presentations, slides, timers, playlists, Bible references) with ergonomic, testable APIs.
- `presenter-server` crate hosts the axum router, exposing JSON/WS endpoints and powering future server-rendered UI slots.
- Async runtime powered by Tokio with tracing instrumentation baked in from day one.
- SQLite chosen for local-first reliability, with migration tooling via SQLx/SeaORM once schema work begins.
- Front-end will attach via Leptos server functions and stream-rendered pages while still allowing pure API consumers (Companion, OSC, MIDI bridges).

## Rationale
- Axum 0.8 (2025 release) provides first-class typed routes, WebSocket support, and tower integration without an enormous runtime footprint, making it ideal for the Intel N100 target.citeturn0search0
- Leptos 0.7 stabilizes server function streaming and island hydration, letting us ship responsive operator/tablet UIs without a separate Node.js runtime—important for keeping deployment simple on the controller.citeturn1search0
- Tokio 1.40 remains the de facto async runtime with proven latency characteristics and ecosystem compatibility (OSC, MIDI, WebSockets).citeturn0search4
- SeaORM + SQLx give us compile-time checked queries and migration tooling while keeping the door open to multi-database strategies later. Both libraries integrate cleanly with Axum/Tokio.

## Consequences
- Rust toolchain becomes a hard dependency; we installed rustup to pin the latest stable (1.90.0) and will keep it updated.
- Initial development focuses on domain modeling and API scaffolding before UI heavy-lifting; Leptos integration will follow once we stabilize the schema and service boundaries.
- Middleware (auth, rate limits, telemetry) will leverage tower layers; we enabled tracing subscriber with env-filter from the start to avoid retrofitting observability later.
- Future integration crates (Resolume WebSockets, Companion variables, MIDI/Bome) will extend the same workspace, benefiting from shared domain types.

## Follow-up
- Define database schema migrations and repository adapters (SeaORM contexts) once storage layer work starts.
- Add structured tracing spans for slide trigger pipeline when implementing outputs.
- Introduce Leptos UI crate and share Tailwind/Design tokens after base API is in place.
