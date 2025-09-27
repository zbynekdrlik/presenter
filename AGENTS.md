# Presenter Agent Handbook

## Purpose
Presenter is a monolithic, production-ready lyrics/Bible/timers display application built exclusively for our church's services. All design and implementation decisions must optimize for our in-house workflows, reliability under high-stakes events, and minimal operator friction.

## Scope & Focus
- Solve only the exact requirements defined by our church production team; avoid generic, publicly-oriented features.
- Prioritize robustness, predictable behavior, and graceful failure handling over breadth of functionality.
- Future revisions to functional scope will be documented separately (e.g., in `docs/functional-needs.md`). Agents should treat that documentation as the source of truth for domain specifics.

## Workflow States
1. **Discovery & Research** – Gather requirements from existing docs or stakeholders. When selecting new tools/frameworks, perform current online research to confirm choices represent state-of-the-art 2025 technology, architecture, and design. Summarize findings and sources in the PR description.
2. **Specification** – Translate requirements into executable tests or clear acceptance criteria before writing production code.
3. **Implementation (TDD)** – Follow red/green/refactor loops. Keep commits small, descriptive, and focused on a single change.
4. **Verification** – Run automated tests and, when applicable, add integration/end-user simulations that mirror service workflows. Update Markdown documentation to stay consistent with behavior.
5. **Review & Iterate** – Use GitHub pull requests for continuous review. Address feedback promptly and document significant decisions (ADRs, PR notes).

## Branching & Review Protocol
- Work exclusively on feature branches named after the task (e.g., `feature/lyrics-presentation`). Never commit directly to `main`; this is a strict rule.
- Create a GitHub pull request immediately after branching to enable ongoing web-based review. Keep the PR description and checklist current.
- Commit and push frequently so reviewers can track incremental progress. Avoid large, monolithic pushes.
- All merges to `main` happen through GitHub PR after explicit user approval. Fast-forwards or direct merges from local machines are forbidden.

## Final Merge Preparation Checklist
- All necessary files are committed; the working tree is clean (no unstaged or untracked changes).
- The feature branch is pushed, and the associated PR reflects the latest commits, notes, and research links.
- Automated tests cover every new behavior, emphasizing functional and end-user workflows. Add integration/end-to-end coverage where relevant.
- Update all Markdown documents affected by the change so architecture and functionality references remain accurate and reflect the chosen 2025 design approach.
- Confirm the implementation aligns with monolithic, high-reliability principles tailored to our church usage and document that rationale in the PR.

## Collaboration & Decision Log
- Capture architectural decisions in numbered ADRs under `docs/adr/` and cross-reference them from PRs when applicable.
- Use GitHub Issues as the primary location for comprehensive feature plans; do not commit temporary planning documents to the repository. Keep plan updates and implementation notes in issue comments.
- Surface open questions or scope uncertainties directly in PRs to secure answers before merge.

## Immediate Next Actions
- Refine documentation to describe the precise church workflows this application must support.
- Evaluate candidate technology stacks for the monolithic controller/rendering application and record findings in ADRs.
- Establish the baseline test harness so TDD can be enforced from the first implementation commit.
