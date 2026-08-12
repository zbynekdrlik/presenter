#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Nested-worktree cargo-fmt smoke check for crates/presenter-ui (#669).
#
# `crates/presenter-ui` is deliberately excluded from the root cargo
# workspace (`exclude = ["crates/presenter-ui"]` in the root Cargo.toml) and
# carries its own `[workspace]` table so cargo treats it as a standalone
# package regardless of ancestor-directory nesting depth — see that table's
# own comment in crates/presenter-ui/Cargo.toml for the full root cause
# (cargo's ancestor-directory workspace-root discovery has zero git/worktree
# boundary awareness and, absent the table, can climb past a nested git
# worktree's own root into an OUTER checkout's Cargo.toml, whose `exclude`
# is relative to that outer root and never matches the nested path).
#
# CI's own `Format` job (`pipeline.yml`) checks out the repo NON-NESTED and
# can never reproduce a regression here — the failure only appears when
# `crates/presenter-ui` is reached through a NESTED git worktree, exactly
# the layout `/autopilot`'s worktree-isolated dispatch uses
# (`.claude/worktrees/<name>/`, default since #317). This script is the
# regression proof CI cannot provide: it creates a throwaway nested git
# worktree INSIDE the current checkout, runs `cargo fmt --check` on
# `presenter-ui` from inside it, asserts the command exits 0, and always
# removes the worktree afterwards (success or failure) so it never leaves a
# stray `git worktree list` entry behind.
#
# Usage: scripts/dev/check-presenter-ui-worktree-fmt.sh
# ============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

WORKTREE_DIR="$ROOT_DIR/.claude/worktrees/smoke-nested-presenter-ui-$$"

cleanup() {
  git -C "$ROOT_DIR" worktree remove --force "$WORKTREE_DIR" >/dev/null 2>&1 || rm -rf "$WORKTREE_DIR"
  git -C "$ROOT_DIR" worktree prune >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> Creating throwaway nested worktree at $WORKTREE_DIR"
git worktree add --detach --quiet "$WORKTREE_DIR" HEAD

echo "==> Running 'cargo fmt --check' inside the nested worktree's crates/presenter-ui"
(cd "$WORKTREE_DIR/crates/presenter-ui" && cargo fmt --check)

echo "PASS: cargo fmt --check succeeded from inside a nested worktree (#669 stays fixed)"
