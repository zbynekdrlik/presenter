# ADR 0003: Dockerized per-branch demo environments

## Status
Proposed

## Context
The current presenter development workflow relies on systemd services and
in-repo scripts that bind directly to host resources (ports, SQLite files).
That approach breaks down when we need to iterate on multiple branches or
dedicated repository working copies in parallel:

- Only one service can bind to port 80 at a time, so demo servers collide.
- Playwright and other integration tests assume exclusive access to the host
  database file and HTTP port.
- Resetting demo data (`refresh-dev-data.sh`) mutates the shared `var/` tree,
  corrupting runs that happen simultaneously in another repo checkout.

We therefore need isolated runtime environments for each branch / repo folder,
while keeping a friendly way for the user to discover and open the demos.

## Decision
We will containerise the presenter stack and codify per-repo demo environments
with Docker Compose. The design has three pillars:

1. **Multi-stage Dockerfile** – builds the presenter binaries (server,
   importer) in a Rust builder stage and ships a slim runtime image that
   exposes `/presenter` as the working root. A baked-in entrypoint runs
   migrations/import jobs before starting the server.

2. **Per-demo Compose stack** – `docker-compose.demo.yml` defines an `app`
   service (presenter server) and a persistent volume for the SQLite database
   plus cached imports. A helper script `scripts/docker/run-demo.sh` launches
   the stack with a Compose project name derived from the repo folder name
   (or explicit override). The project name becomes the namespace for
   containers, volumes, and network to avoid clashes. Ports are also isolated:
   by default we expose the HTTP interface on `8080` inside the container and
   publish it on a high host port that is calculated from the Compose project
   hash, but the script allows explicitly picking a port.

   Each demo writes a small manifest (`${XDG_DATA_HOME:-$HOME/.local/share}/presenter-demos/manifests/<project>.json`) with
   metadata: display name, host port, repository path, and last refresh time.

3. **Landing gateway on port 80** – a lightweight Node/Express service runs in
   a separate Compose stack (`docker-compose.gateway.yml`). It watches the
   manifests directory and renders an index page listing every active demo with
   buttons to open the operator UI, Bible UI, and stage preview. The gateway
   can be started once (systemd or `docker compose up -d`) and remains stable
   while demos come and go.

Playwright and the rest of the test suite will rely on the same container
infrastructure: test commands spin up an ephemeral demo stack (project name
includes a random suffix), run migrations/imports inside the container, execute
browser tests against the published port, then tear everything down. Because
volumes and ports are namespaced per project, concurrent playwright runs do
not interfere.

## Consequences
- Repository root gains a Dockerfile, `docker-compose.demo.yml`, gateway stack,
  and helper scripts under `scripts/docker/`.
- `AGENTS.md` & README will be updated with new workflows for starting/stopping
  demos and running tests in Dockerised mode.
- Systemd units for the old bare-metal demo (`presenter-demo.service`) become
  optional / deprecated.
- CI can reuse the docker tooling to spin up demo environments in a predictable
  way.
- Developers must have Docker / Docker Compose V2 installed.

## Alternatives considered
- **Multiple systemd services with different ports** – rejected because it
  still shares the same SQLite files and requires manual port management.
- **Kubernetes minikube** – overkill for a single-node demo setup and would add
  significant onboarding friction.
- **Docker "compose override" only** – we opted for helper scripts so that the
  workflow is one command per repo folder and to maintain the manifests for the
  landing page.
