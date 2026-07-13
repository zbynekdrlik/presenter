import { test, expect, type APIRequestContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #546 — the PP outage, reproduced against REAL NDI discovery.
//
// At PP the operator mapped `cgpp → RESOLUME-PP (cg-obs)` and activated it. The
// activation SUCCEEDED (a silent broadcaster is not an error — #448), the stage stayed
// blank, and the UI said nothing at all. The name simply was not on the network; only
// `STREAM-PP (stream)` was. Hours were spent on the encoder chain before a log line
// revealed it.
//
// The libndi-free lane can only prove the "server is blind" path (video-source-status
// .spec.ts). This file is the one that reproduces the actual defect: a mapped name that
// genuinely is NOT on the air, while a DIFFERENT name genuinely IS — and asserts the
// operator can now see both facts side by side.
//
// Tags: @video-codec routes to the real-Chrome project; @synthetic-ndi selects it into
// the self-hosted `e2e-ndi` lane, which publishes "<host> (PRESENTER-TEST)" before
// Playwright starts.
// ─────────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

/** The synthetic source the lane publishes. Discovery needs a few seconds after a
 * fresh server start, so poll rather than ask once. */
async function discoverSynthetic(
  request: APIRequestContext,
): Promise<{ name: string }> {
  for (let i = 0; i < 30; i++) {
    const resp = await request.get(new URL("/ndi/sources", baseURL).toString());
    if (resp.ok()) {
      const list = await resp.json();
      if (Array.isArray(list)) {
        const found = list.find((s: { name: string }) =>
          s.name.includes("(PRESENTER-TEST)"),
        );
        if (found) return found;
      }
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(
    "the synthetic NDI source never appeared — the e2e-ndi lane must publish it",
  );
}

async function createAndActivate(
  request: APIRequestContext,
  label: string,
  ndiName: string,
): Promise<{ id: string }> {
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label, ndiName } },
  );
  expect(created.status()).toBe(200);
  const src = await created.json();
  const activated = await request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  // Exactly as at PP: activating a name nobody is broadcasting SUCCEEDS. That is the
  // whole trap — nothing fails, and nothing tells you.
  expect(activated.status()).toBe(200);
  return src;
}

test("a mapped NDI name that is not on the network is reported as not found @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSynthetic(request);
  // The PP shape: map a name nobody broadcasts, while a real one IS on the air.
  const ghost = await createAndActivate(request, "cgpp", "GHOST-SOURCE (nope)");

  const status = await request.get(
    new URL("/integrations/video-sources/status", baseURL).toString(),
  );
  expect(status.status()).toBe(200);
  const body = await status.json();
  expect(body.ndiAvailable).toBe(true);
  expect(
    body.discovered,
    "the synthetic source really is on the network",
  ).toContain(synthetic.name);
  expect(
    body.sources.find((s: any) => s.id === ghost.id).state,
    "the mapped name is nowhere on the network — the server must say so",
  ).toBe("not-found");

  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 60_000 });

  const badge = page.locator(
    `[data-role="video-source-status"][data-source-id="${ghost.id}"]`,
  );
  await expect(badge).toHaveText("Not found on the network", { timeout: 30_000 });
  await expect(badge).toHaveAttribute("data-state", "not-found");

  const hint = page.locator(
    `[data-role="video-source-hint"][data-source-id="${ghost.id}"]`,
  );
  await expect(hint).toBeVisible();

  // …and the operator can SEE what is actually on the air, right there — which is what
  // makes "RESOLUME-PP vs STREAM-PP" obvious instead of a two-hour investigation.
  await expect(
    page.locator('[data-role="ndi-discovered-name"]', {
      hasText: "(PRESENTER-TEST)",
    }),
  ).toBeVisible({ timeout: 30_000 });
});

test("a mapped NDI name that IS broadcasting goes Live @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSynthetic(request);
  const source = await createAndActivate(request, "synthetic", synthetic.name);

  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 60_000 });

  const badge = page.locator(
    `[data-role="video-source-status"][data-source-id="${source.id}"]`,
  );
  // Pipeline start (up to ~8s) + one 5s status poll, with margin.
  await expect(badge).toHaveText("Live", { timeout: 40_000 });
  await expect(badge).toHaveAttribute("data-state", "live");

  // A working source needs no instructions.
  await expect(
    page.locator(`[data-role="video-source-hint"][data-source-id="${source.id}"]`),
  ).toHaveCount(0);

  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
});
