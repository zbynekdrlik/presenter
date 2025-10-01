# Presenter Agent Handbook

## Purpose
Presenter is a monolithic, production-ready lyrics/Bible/timers display application built exclusively for our church's services. All design and implementation decisions must optimize for our in-house workflows, reliability under high-stakes events, and minimal operator friction.

## Scope & Focus
- Solve only the exact requirements defined by our church production team; avoid generic, publicly-oriented features.
- Prioritize robustness, predictable behavior, and graceful failure handling over breadth of functionality.
- Future revisions to functional scope will be documented separately (e.g., in `docs/functional-needs.md`). Agents should treat that documentation as the source of truth for domain specifics.

## Design Iteration Policy
- We are still in the **initial design round**. Treat every redesign request as a greenfield effort aimed at delivering the first correct architecture; never anchor on prior incorrect drafts or partial implementations.
- Do **not** preserve compatibility with previous UI code, APIs, database snapshots, or scripts that stem from earlier flawed iterations. Remove or rewrite them freely in favor of the proper design.
- When refactoring, prefer clean rebuilds over incremental patches that keep legacy artifacts. Strive for the clearest expression of the current requirements, even if that means deleting large portions of prior work.

## Tooling & Dependencies
- Install any required tool, utility, package, framework, or application immediately using `sudo apt install` (or the appropriate package manager) instead of attempting workarounds or waiting for approval.
- If Ubuntu's default repositories lack a sufficiently recent version, research state-of-the-art 2025 sources (official PPAs, vendor repos, direct installers) and install the newest stable release that satisfies project requirements. Document the chosen source in the PR.
- Preference is always for the latest stable versions of crucial frameworks; do not proceed with outdated tooling unless explicitly directed by the user.

## Runtime Environments
- Maintain three installations on this controller: development, testing, and production. Keep dev updated immediately after each approved PR so the user can evaluate changes quickly.
- Testing environment mirrors production configuration for validation prior to promotion. Automate deployment from the main branch once changes pass review.
- Production environment is updated only after explicit user approval; ensure zero-downtime handoff and rollback plan.
- Always prioritize the fastest path for the user to test changes on the development instance, then promote to production for other operators once sign-off is received.
- Docker is now the primary demo platform. Keep the gateway (`./scripts/docker/run-gateway.sh`) running on port 80 with manifests mounted from `${XDG_DATA_HOME:-$HOME/.local/share}/presenter-demos/manifests`. For each feature branch checkout, launch or refresh its dedicated stack via `./scripts/docker/run-demo.sh`. **Agents are responsible for starting, stopping, and keeping these demos current without prompting the user.** Ensure only one demo per repository folder is active—`run-demo.sh` already stops stale containers for the same repo—and immediately tear down any legacy systemd or bare-metal presenter services so they never conflict with Docker-managed demos.
- Always invoke `./scripts/dev/verify-and-refresh.sh` with `sudo -E` so Docker commands succeed while tests still run as your user; the helper now enforces this requirement at startup.

## Database Management (Pre-release)
- We are still before our first public release. Treat the schema as mutable: **never add incremental migrations**. Instead, evolve the single initial migration (or equivalent schema definition) directly whenever the data model changes.
- After each schema adjustment, rebuild the SQLite database from scratch and rerun the full ingestion pipeline (ProPresenter + Bible imports) so development, testing, and production remain aligned. Do not attempt in-place migrations or partial data preservation at this stage.
- Keep scripts such as `scripts/dev/refresh-dev-data.sh` up to date so they create a clean database and repopulate it automatically after every change.

## Workflow States
1. **Discovery & Research** – Gather requirements from existing docs or stakeholders. When selecting new tools/frameworks, perform current online research to confirm choices represent state-of-the-art 2025 technology, architecture, and design. Summarize findings and sources in the PR description.
2. **Specification** – Translate requirements into executable tests or clear acceptance criteria before writing production code.
3. **Implementation (TDD)** – Follow red/green/refactor loops. Keep commits small, descriptive, and focused on a single change.
4. **Verification** – Run automated tests and, when applicable, add integration/end-user simulations that mirror service workflows. Ensure the suites include a health check that pings the always-on port-80 demo instance across every active interface (loopback, LAN, Zerotier) and that they fail if the landing page card for the current branch shows a stale `data-updated-at` timestamp. Update Markdown documentation to stay consistent with behavior. Use `./scripts/dev/verify-and-refresh.sh --force` so cargo tests, Playwright, the demo rebuild, and the gateway refresh happen together before reporting progress—the script stops at the first failure, so fix issues and rerun until it completes successfully.
5. **Review & Iterate** – Use GitHub pull requests for continuous review. Address feedback promptly and document significant decisions (ADRs, PR notes).

## Branching & Review Protocol
- Work exclusively on feature branches named after the task (e.g., `feature/lyrics-presentation`). Never commit directly to `main`; this is a strict rule.
- Create a GitHub pull request immediately after branching to enable ongoing web-based review. Keep the PR description and checklist current.
- Commit and push frequently so reviewers can track incremental progress. Avoid large, monolithic pushes.
- Never declare work “done” or report progress to the user until the full automated suites have just been executed locally and completed without failures. Minimum required commands: `cargo test` and `npm run test:playwright`, both of which must include the demo-server reachability checks.
- All merges to `main` happen through GitHub PR after explicit user approval. Fast-forwards or direct merges from local machines are forbidden.
- Agents must never merge pull requests. Only the user merges PRs after review. Agents may prepare branches and PRs but must await explicit user approval for the merge.

## Final Merge Preparation Checklist
- All necessary files are committed; the working tree is clean (no unstaged or untracked changes).
- The feature branch is pushed, and the associated PR reflects the latest commits, notes, and research links.
- Automated tests cover every new behavior, emphasizing functional and end-user workflows. Add integration/end-to-end coverage where relevant.
- Update all Markdown documents affected by the change so architecture and functionality references remain accurate and reflect the chosen 2025 design approach.
- Confirm the implementation aligns with monolithic, high-reliability principles tailored to our church usage and document that rationale in the PR.

## Collaboration & Decision Log
- Capture architectural decisions in numbered ADRs under `docs/adr/` and cross-reference them from PRs when applicable.
- Use GitHub Issues as the primary location for comprehensive feature plans; do not commit temporary planning documents to the repository. Keep plan updates and implementation notes in issue comments.
- Surface open questions or scope uncertainties directly to the project owner (user) in the active conversation; PR comments alone are insufficient.

## Immediate Next Actions
- Refine documentation to describe the precise church workflows this application must support.
- Evaluate candidate technology stacks for the monolithic controller/rendering application and record findings in ADRs.
- Establish the baseline test harness so TDD can be enforced from the first implementation commit.
