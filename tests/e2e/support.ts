import { spawn, type ChildProcessWithoutNullStreams } from "child_process";
import { once } from "events";
import http from "http";
import path from "path";
import type { AddressInfo } from "net";
import type { TestInfo } from "@playwright/test";
import { expect, type Locator, type Page } from "@playwright/test";

/**
 * Subscribe to console events on `page` and append error/warning messages to
 * `errors`. Pass the array to `expect(errors).toEqual([])` at the end of the
 * test to assert a clean browser console.
 *
 * Mirrors the inline pattern used across NDI specs (e.g. ndi-webrtc.spec.ts)
 * but extracted here so concurrent-context tests (ndi-webrtc-fanout.spec.ts)
 * don't have to duplicate the setup.
 */
/**
 * Wait for the lite NDI stage page to be the loaded document.
 *
 * EXPERIMENT (#379): while the ndi-fullscreen layout is active, GET /stage
 * 303-redirects to /stage/lite — a plain-JS WHEP player with no WASM app
 * (the 1GB Vestel TVs stall on the WASM page; VDO.Ninja-style plain JS has
 * played on the same TVs for years). Specs that previously waited for the
 * WASM shell (`body[data-wasm-ready="true"]` + layout-code) on the NDI
 * layout wait for the lite marker instead.
 */
export async function waitForNdiLitePage(page: Page): Promise<void> {
  await page.waitForSelector('body[data-ndi-lite="true"]', {
    timeout: 30_000,
  });
}

export function attachConsoleErrorCollector(page: Page, errors: string[]): void {
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      errors.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => {
    errors.push(`[pageerror] ${err.message}`);
  });
}

/**
 * Poll `<video data-role="ndi-video">` (or another `selector`) until
 * `videoWidth > 0` AND `readyState >= HAVE_CURRENT_DATA (2)`. Polls every
 * 100 ms for up to 12 s (120 iterations) — chosen to match the existing
 * "NdiVideo videoWidth resolves above zero" test in ndi-webrtc.spec.ts but
 * with extra headroom for a second concurrent consumer.
 *
 * Throws if the element is not found or the video never reaches a playable
 * state within the timeout.
 */
export async function waitForVideoReady(
  page: Page,
  selector: string,
): Promise<void> {
  const ok = await page.locator(selector).evaluate(
    async (el: HTMLVideoElement) => {
      for (let i = 0; i < 120; i++) {
        if (el.videoWidth > 0 && el.readyState >= 2) return true;
        await new Promise((r) => setTimeout(r, 100));
      }
      return el.videoWidth > 0 && el.readyState >= 2;
    },
  );
  expect(
    ok,
    `${selector}: videoWidth never exceeded 0 within 12s (readyState < HAVE_CURRENT_DATA)`,
  ).toBe(true);
}

/**
 * Discover available NDI sources via `/ndi/sources`, pick the first one, and
 * activate it as a video source row so it has a UUID that the WHEP endpoint
 * accepts. Returns the activated video-source UUID, or `null` if no NDI
 * source is discoverable on this runner (caller should `test.skip()`).
 *
 * Mirrors the inline pattern used by ndi-webrtc.spec.ts and
 * ndi-webrtc-recovery.spec.ts — extracted here so the fanout test can reuse
 * it without duplication.
 *
 * NOTE: callers that need a full `activate` (pipeline ready) should call
 * `POST /integrations/video-sources/:id/activate` themselves after this
 * returns. This helper only creates the DB row (POST /integrations/video-sources)
 * without activating, because the fanout test activates as part of its own
 * flow to avoid races between deactivate/create/activate.
 */
export async function activateFirstNdiSource(server: {
  baseUrl: string;
}): Promise<string | null> {
  // Discover sources from the live NDI manager.
  let discovered: Array<{ name: string }>;
  try {
    const resp = await fetch(`${server.baseUrl}/ndi/sources`);
    if (!resp.ok) return null;
    const body = await resp.json();
    if (!Array.isArray(body) || body.length === 0) return null;
    discovered = body as Array<{ name: string }>;
  } catch {
    return null;
  }

  const ndiName = discovered[0].name;

  // Create a video_source row for this NDI broadcaster.
  const createResp = await fetch(
    `${server.baseUrl}/integrations/video-sources`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ label: "fanout-test", ndiName }),
    },
  );
  if (!createResp.ok) return null;
  const src = (await createResp.json()) as { id: string };

  // Activate so the pipeline starts.
  const activateResp = await fetch(
    `${server.baseUrl}/integrations/video-sources/${src.id}/activate`,
    { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" },
  );
  if (!activateResp.ok) return null;

  return src.id;
}

export const REPO_ROOT = process.cwd();

/**
 * Assert that a container uses a two-column layout with left and right
 * columns rendered side-by-side at the expected widths.
 */
export async function assertTwoColumnLayout(
  container: Locator,
  leftColumn: Locator,
  rightColumn: Locator,
  options: {
    expectedLeftWidth?: number;
    leftWidthTolerance?: number;
    expectedDisplay?: string;
  } = {},
) {
  const {
    expectedLeftWidth = 320,
    leftWidthTolerance = 10,
    expectedDisplay = "grid",
  } = options;

  // Container uses expected display mode
  const display = await container.evaluate(
    (el) => window.getComputedStyle(el).display,
  );
  expect(display).toBe(expectedDisplay);

  // Get bounding boxes
  const leftBox = await leftColumn.boundingBox();
  const rightBox = await rightColumn.boundingBox();
  expect(leftBox).toBeTruthy();
  expect(rightBox).toBeTruthy();
  if (!leftBox || !rightBox) return;

  // Side-by-side: same vertical position (not stacked)
  expect(Math.abs(leftBox.y - rightBox.y)).toBeLessThan(5);

  // Left column width ~expected
  expect(leftBox.width).toBeGreaterThan(expectedLeftWidth - leftWidthTolerance);
  expect(leftBox.width).toBeLessThan(expectedLeftWidth + leftWidthTolerance);

  // No overlap
  expect(rightBox.x).toBeGreaterThanOrEqual(leftBox.x + leftBox.width - 1);

  // Right column fills remaining space
  expect(rightBox.width).toBeGreaterThan(leftBox.width);
}

function stableHash(input: string): number {
  let hash = 0;
  for (let i = 0; i < input.length; i += 1) {
    hash = (hash * 31 + input.charCodeAt(i)) >>> 0;
  }
  return hash;
}

export type ServerHandle = {
  process: ChildProcessWithoutNullStreams;
  port: number;
  stop: () => Promise<void>;
};

export type MockResolumeHandle = {
  port: number;
  setOnline: (online: boolean) => void;
  close: () => Promise<void>;
};

export type MockAbleSetHandle = {
  port: number;
  /** update the active song name returned by the mock */
  setActiveSong: (name: string, id?: string, order?: number) => void;
  close: () => Promise<void>;
};

export type TestConfig = {
  workerIndex: number;
  port: number;
  dbUrl: string;
  baseURL: string;
  oscPort: number;
};

export function deriveTestConfig(testInfo: TestInfo): TestConfig {
  const workerIndex = testInfo.workerIndex ?? 0;
  const basePort = Number.parseInt(
    process.env.PRESENTER_PORT_BASE ?? "18999",
    10,
  );
  const scopeKey = testInfo.file ?? testInfo.title ?? `worker-${workerIndex}`;
  const fileOffset = stableHash(scopeKey) % 50;
  const port = basePort + workerIndex * 100 + fileOffset;
  const explicitDbUrl = process.env.PRESENTER_DB_URL;
  const defaultDbPath = path.join(
    REPO_ROOT,
    "var",
    "tmp",
    `presenter_e2e_${workerIndex}.db`,
  );
  const dbUrl = explicitDbUrl ?? `sqlite://${defaultDbPath}`;
  const baseURL = `http://127.0.0.1:${port}`;
  const oscPort = port + 1;
  return { workerIndex, port, dbUrl, baseURL, oscPort };
}

export function runShell(
  command: string,
  extraEnv: NodeJS.ProcessEnv = {},
): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn("bash", ["-lc", command], {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        ...extraEnv,
      },
      stdio: "inherit",
    });

    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`Command failed (${code ?? "unknown"}): ${command}`));
      }
    });
  });
}

const DEFAULT_LIBRARY_ROOT =
  process.env.PRESENTER_LIBRARY_ROOT ??
  path.join(REPO_ROOT, "data", "libraries");

export async function refreshDevData(
  dbUrl: string,
  root = DEFAULT_LIBRARY_ROOT,
) {
  const bibleDir = path.join(REPO_ROOT, "data", "bibles");
  await runShell(
    `PRESENTER_DB_URL=${dbUrl} ./scripts/dev/refresh-dev-data.sh "${root}"`,
    {
      PRESENTER_DB_URL: dbUrl,
      PRESENTER_BIBLE_KJV: path.join(bibleDir, "kjv.usfm.zip"),
      PRESENTER_BIBLE_SEB: path.join(bibleDir, "seb.bbl.mybible.zip"),
      PRESENTER_BIBLE_ROHACEK: path.join(bibleDir, "rohacek.bbl.mybible.zip"),
      PRESENTER_BIBLE_SEVP: path.join(bibleDir, "sevp.obohu.mybible.zip"),
    },
  );
}

async function waitForServerReady(baseURL: string, timeoutMs = 240_000) {
  const startedAt = Date.now();
  const healthUrl = new URL("/healthz", baseURL).toString();

  while (Date.now() - startedAt < timeoutMs) {
    try {
      const response = await fetch(healthUrl, {
        signal: AbortSignal.timeout(5_000),
      });
      if (response.ok) {
        return;
      }
    } catch {
      // retry after delay
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }

  throw new Error(
    `Presenter server did not become ready within ${timeoutMs}ms`,
  );
}

export async function startTestServer(
  port: number,
  dbUrl: string,
  oscPort?: number,
): Promise<ServerHandle> {
  const env = {
    ...process.env,
    PRESENTER_DB_URL: dbUrl,
    PRESENTER_PORT: String(port),
    ...(oscPort ? { PRESENTER_OSC_LISTEN_PORT: String(oscPort) } : {}),
    PRESENTER_ANDROID_ADB_BIN: process.env.PRESENTER_ANDROID_ADB_BIN ?? "true",
    RUST_LOG:
      process.env.RUST_LOG ?? "presenter_server=info,tower_http=warn,sqlx=warn",
  };

  // Use pre-built binary if available (CI builds binaries first), otherwise fall back to cargo run
  const debugBinary = path.join(
    REPO_ROOT,
    "target",
    "debug",
    "presenter-server",
  );
  const releaseBinary = path.join(
    REPO_ROOT,
    "target",
    "release",
    "presenter-server",
  );

  // When BOTH binaries exist (local dev), pick the NEWER one by mtime — a
  // stale target/release/ binary silently shadowing a fresh target/debug/
  // build makes e2e runs exercise OLD code and report false verdicts. On CI
  // only the freshly-built release binary exists, so behavior is unchanged.
  const fs = require("fs");
  const mtimeOf = (p: string): number => {
    try {
      return fs.statSync(p).mtimeMs;
    } catch {
      return -1;
    }
  };
  const releaseMtime = mtimeOf(releaseBinary);
  const debugMtime = mtimeOf(debugBinary);
  let command: string;
  if (releaseMtime >= 0 && releaseMtime >= debugMtime) {
    command = releaseBinary;
  } else if (debugMtime >= 0) {
    command = debugBinary;
  } else {
    // Fall back to cargo run if no pre-built binary exists
    command = `cargo run -p presenter-server`;
  }
  console.log(`[e2e] test server binary: ${command}`);

  const processHandle = spawn(
    "bash",
    [
      "-lc",
      `PRESENTER_DB_URL=${dbUrl} PRESENTER_PORT=${port} ${oscPort ? `PRESENTER_OSC_LISTEN_PORT=${oscPort} ` : ""}${command}`,
    ],
    {
      cwd: REPO_ROOT,
      env,
      stdio: "inherit",
    },
  );

  await waitForServerReady(`http://127.0.0.1:${port}`);

  return {
    process: processHandle,
    port,
    stop: async () => {
      processHandle.kill("SIGTERM");
      await once(processHandle, "exit");
    },
  };
}

export async function stopServer(handle?: ServerHandle) {
  if (!handle) return;
  handle.process.kill("SIGTERM");
  await once(handle.process, "exit");
}

export async function startMockResolume(): Promise<MockResolumeHandle> {
  let online = true;

  const server = http.createServer((req, res) => {
    const { method, url } = req;
    if (!url) {
      res.statusCode = 400;
      return res.end("bad request");
    }

    if (!online) {
      res.statusCode = 503;
      return res.end("resolume offline");
    }

    if (method === "GET" && url.startsWith("/api/v1/composition")) {
      res.writeHead(200, { "content-type": "application/json" });
      const body = {
        layers: [
          {
            clips: [
              clip(100, "#main-a", 1),
              clip(101, "#main-b", 2),
              clip(200, "#translate-a", 10),
              clip(201, "#translate-b", 20),
              clip(300, "#bible-a", 30),
              clip(301, "#bible-b", 31),
              clip(400, "#bible-translate-a", 40),
              clip(401, "#bible-translate-b", 41),
              clip(500, "#bible-clear", undefined),
              clip(600, "#timer", 60),
              clip(700, "#song-name", undefined),
              clip(701, "#band-name", undefined),
            ],
          },
        ],
      };
      const payload = JSON.stringify(body);
      res.end(payload);
      return;
    }

    if (method === "PUT" && url.startsWith("/api/v1/parameter/by-id/")) {
      res.statusCode = 200;
      res.end();
      return;
    }

    if (
      method === "POST" &&
      url.startsWith("/api/v1/composition/clips/by-id/")
    ) {
      res.statusCode = 200;
      res.end();
      return;
    }

    res.statusCode = 404;
    res.end("not found");
  });

  function clip(id: number, name: string, param?: number) {
    const sourceparams = param
      ? {
          text: {
            valuetype: "ParamText",
            id: param,
          },
        }
      : {};

    return {
      id,
      name: { value: name },
      video: { sourceparams },
    };
  }

  await new Promise<void>((resolve, reject) => {
    server.listen(0, "127.0.0.1", (err?: Error) => {
      if (err) reject(err);
      else resolve();
    });
  });

  const address = server.address() as AddressInfo;

  return {
    port: address.port,
    setOnline: (value: boolean) => {
      online = value;
    },
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((err) => {
          if (err) reject(err);
          else resolve();
        });
      }),
  };
}

/**
 * Assert that the version label on the current page exists, has the expected
 * format, and matches the backend `/healthz` response.
 *
 * Format: `v<major>.<minor>.<patch>(-dev.<n>)?( (<channel>))?`
 * Examples: `v0.4.52`, `v0.4.52 (dev)`, `v0.4.52-dev.3 (dev)`
 *
 * Frontend version MUST equal `/healthz` `version` field — single source of
 * truth. Channel suffix appears only for non-release builds.
 */
export async function assertVersionLabel(
  page: Page,
  baseURL: string,
): Promise<void> {
  const versionEl = page.locator('[data-testid="version"]').first();
  await expect(versionEl).toBeVisible({ timeout: 10_000 });
  // VersionLabel renders an empty <span> immediately; the version text
  // populates asynchronously after /healthz resolves. Wait for non-empty
  // text before reading, otherwise the assertion races with the WASM fetch.
  await expect(versionEl).not.toHaveText("", { timeout: 10_000 });

  const text = (await versionEl.textContent())?.trim() ?? "";
  expect(text).toMatch(/^v\d+\.\d+\.\d+(-dev\.\d+)?(\s\(\w+\))?$/);

  const healthRes = await page.request.get(
    new URL("/healthz", baseURL).toString(),
  );
  const health = (await healthRes.json()) as {
    version: string;
    channel: string;
  };
  const expected =
    health.channel === "release" || health.channel === ""
      ? `v${health.version}`
      : `v${health.version} (${health.channel})`;
  expect(text).toBe(expected);
}

export async function startMockAbleSet(): Promise<MockAbleSetHandle> {
  let activeId = "song-1";
  let activeName = "148 Vrat ma spat";
  let activeOrder: number | undefined = 5;

  const server = http.createServer((req, res) => {
    const { method, url } = req;
    if (!url) {
      res.statusCode = 400;
      return res.end("bad request");
    }
    if (method === "GET" && url.startsWith("/api/setlist")) {
      res.writeHead(200, { "content-type": "application/json" });
      const payload = {
        activeSongId: activeId,
        songs: [
          {
            id: activeId,
            meta: { name: activeName, raw: activeName },
            internalMeta:
              activeOrder != null ? { order: activeOrder } : undefined,
          },
        ],
      };
      res.end(JSON.stringify(payload));
      return;
    }
    res.statusCode = 404;
    res.end("not found");
  });

  await new Promise<void>((resolve, reject) => {
    server.listen(0, "127.0.0.1", (err?: Error) =>
      err ? reject(err) : resolve(),
    );
  });
  const address = server.address() as AddressInfo;
  return {
    port: address.port,
    setActiveSong: (name: string, id = "song-1", order = 0) => {
      activeName = name;
      activeId = id;
      activeOrder = order;
    },
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((err) => (err ? reject(err) : resolve()));
      }),
  };
}
