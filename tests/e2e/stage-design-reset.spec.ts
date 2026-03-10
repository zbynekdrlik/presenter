import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("customize design then reset restores defaults", async ({ request }) => {
  const layout = "worship-snv";

  // Get the default design first
  const defaultResp = await request.get(
    new URL(`/stage/design/${layout}`, baseURL).toString(),
  );
  expect(defaultResp.ok()).toBeTruthy();
  const defaultDesign = await defaultResp.json();

  // Modify the design (change a box position)
  const modified = JSON.parse(JSON.stringify(defaultDesign));
  if (modified.boxes && modified.boxes.length > 0) {
    modified.boxes[0].x = 99.0;
    modified.boxes[0].y = 99.0;
  }

  const putResp = await request.put(
    new URL(`/stage/design/${layout}`, baseURL).toString(),
    { data: modified },
  );
  expect(putResp.status()).toBe(204);

  // Verify the modification persisted
  const modifiedResp = await request.get(
    new URL(`/stage/design/${layout}`, baseURL).toString(),
  );
  expect(modifiedResp.ok()).toBeTruthy();
  const modifiedCheck = await modifiedResp.json();
  if (modifiedCheck.boxes && modifiedCheck.boxes.length > 0) {
    expect(modifiedCheck.boxes[0].x).toBe(99.0);
  }

  // Reset the design
  const resetResp = await request.post(
    new URL(`/stage/design/${layout}/reset`, baseURL).toString(),
  );
  expect(resetResp.ok()).toBeTruthy();
  const resetDesign = await resetResp.json();

  // Verify reset restored defaults (box positions should be back to original)
  expect(resetDesign.layoutCode).toBe(layout);
  if (resetDesign.boxes && resetDesign.boxes.length > 0) {
    expect(resetDesign.boxes[0].x).not.toBe(99.0);
  }

  // Re-fetch to confirm persistence of reset
  const afterResetResp = await request.get(
    new URL(`/stage/design/${layout}`, baseURL).toString(),
  );
  expect(afterResetResp.ok()).toBeTruthy();
  const afterReset = await afterResetResp.json();
  expect(afterReset.layoutCode).toBe(layout);
  if (afterReset.boxes && afterReset.boxes.length > 0) {
    expect(afterReset.boxes[0].x).not.toBe(99.0);
  }
});
