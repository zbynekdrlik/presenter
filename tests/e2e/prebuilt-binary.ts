import { statSync } from "fs";
import path from "path";

/**
 * Resolve a prebuilt workspace binary for the E2E harness (#802).
 *
 * Presenter is Tier-0: Rust compiles in CI only. The harness therefore runs
 * the prebuilt binary (the CI `build-artifacts` artifact, or a fresh CI build)
 * and NEVER falls back to a local compile on its own — that fallback ran inside
 * node, invisible to the Bash-level Tier-0 hook, and silently compiled the
 * workspace. A missing binary is a loud, actionable error. Compiling locally
 * is an explicit opt-in: `PRESENTER_ALLOW_LOCAL_CARGO=1`.
 *
 * Only this module may spell a cargo compile command; the #802 guard
 * (`scripts/ci/test_no_local_cargo_fallback.py`) fails CI otherwise.
 */
export interface CargoTarget {
  /** Binary file name under `target/<profile>/`. */
  bin: string;
  /** Cargo package that owns the binary. */
  pkg: string;
  /** Cargo features needed to build it (opt-in compile only). */
  features?: string[];
  /** Profiles to accept, in preference order when mtimes tie. */
  profiles?: Array<"release" | "debug">;
}

export const ALLOW_LOCAL_CARGO_ENV = "PRESENTER_ALLOW_LOCAL_CARGO";

function mtimeOf(p: string): number {
  try {
    return statSync(p).mtimeMs;
  } catch {
    return -1;
  }
}

function localCargoCommand(target: CargoTarget): string {
  const features = target.features?.length
    ? ` --features ${target.features.join(",")}`
    : "";
  return `cargo run -p ${target.pkg}${features} --bin ${target.bin}`;
}

/**
 * Return a shell command that runs `target`: the NEWEST existing prebuilt
 * binary among the accepted profiles (a stale `target/release` must not shadow
 * a fresher `target/debug`), or — only with the opt-in — a local cargo run.
 * Throws with a `gh run download` recipe otherwise.
 */
export function resolvePrebuiltCommand(
  repoRoot: string,
  target: CargoTarget,
): string {
  const profiles = target.profiles ?? ["release", "debug"];
  let best: { file: string; mtime: number } | undefined;
  for (const profile of profiles) {
    const file = path.join(repoRoot, "target", profile, target.bin);
    const mtime = mtimeOf(file);
    if (mtime >= 0 && (!best || mtime > best.mtime)) {
      best = { file, mtime };
    }
  }
  if (best) return best.file;

  if (process.env[ALLOW_LOCAL_CARGO_ENV] === "1") {
    console.warn(
      `[e2e] ${ALLOW_LOCAL_CARGO_ENV}=1 — compiling ${target.bin} locally (Tier-0 opt-in)`,
    );
    return localCargoCommand(target);
  }

  const expected = profiles
    .map((p) => path.join("target", p, target.bin))
    .join(" or ");
  throw new Error(
    [
      `[e2e] prebuilt binary '${target.bin}' not found (expected ${expected}).`,
      `Presenter is Tier-0: Rust compiles in CI only, the E2E harness never builds locally (#802).`,
      `Download the CI build artifact for your commit and place it in target/release/:`,
      `  gh run download <run-id> -n build-artifacts -D _artifacts`,
      `  mkdir -p target/release && cp _artifacts/* target/release/ && chmod +x target/release/*`,
      `(find <run-id> with: gh run list -w pipeline.yml -b dev -L 5)`,
      `Explicit local-compile opt-in (Tier-1/2 boxes only): ${ALLOW_LOCAL_CARGO_ENV}=1`,
    ].join("\n"),
  );
}
