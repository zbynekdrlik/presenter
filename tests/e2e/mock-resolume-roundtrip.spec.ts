/**
 * E2E test for issue #279: dev's outbound Resolume calls must hit the
 * embedded mock listener on 127.0.0.1:8091, NOT a real Resolume Arena.
 *
 * Skipped on prod builds (channel === "release") because the mock
 * isn't compiled into prod binaries.
 *
 * The mock's request log is on 127.0.0.1:8091/__mock/log. The test
 * runs from the CI runner which lives on the same machine as the dev
 * server, so 127.0.0.1 resolves to the dev server.
 */

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("dev resolume calls hit the mock, not real arena", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Skip on prod (release channel) — mock-integrations feature isn't built.
  const healthRes = await request.get(new URL("/healthz", baseURL).toString());
  expect(healthRes.ok()).toBeTruthy();
  const health = (await healthRes.json()) as { channel: string };
  if (health.channel === "release") {
    test.skip(true, "mock-integrations not built into release binaries");
    return;
  }

  // Verify the embedded mock is reachable.
  // If not, skip (e.g. binary built without mock-integrations feature).
  const mockLogUrl = "http://127.0.0.1:8091/__mock/log";
  let beforeRes: import("@playwright/test").APIResponse;
  try {
    beforeRes = await request.get(mockLogUrl, { timeout: 5_000 });
  } catch {
    test.skip(true, "mock-resolume not reachable — binary missing mock-integrations feature");
    return;
  }
  if (!beforeRes.ok()) {
    test.skip(true, "mock-resolume returned non-OK — not available in this build");
    return;
  }

  const beforeEntries = (await beforeRes.json()) as Array<{
    mock: string;
    method: string;
    path: string;
  }>;
  const beforeCount = beforeEntries.length;

  // Open the operator UI settings page so console capture is active.
  await page.goto(new URL("/ui/operator/settings", baseURL).toString());
  await page.waitForLoadState("networkidle");

  // Create a Resolume host pointing at the embedded mock (127.0.0.1:8091).
  // The fresh test DB has no hosts, so we add one here.
  const createRes = await request.post(
    new URL("/integrations/resolume/hosts", baseURL).toString(),
    {
      data: {
        label: "E2E mock roundtrip",
        host: "127.0.0.1",
        port: 8091,
        isEnabled: true,
      },
      headers: { "Content-Type": "application/json" },
    },
  );
  expect(createRes.ok()).toBeTruthy();
  const created = (await createRes.json()) as { id: string };
  const hostId = created.id;

  // POST test-connection — this causes presenter-server to call
  // GET /api/v1/composition on 127.0.0.1:8091, which the embedded mock
  // records in its request log.
  const testRes = await request.post(
    new URL(`/integrations/resolume/hosts/${hostId}/test`, baseURL).toString(),
  );
  expect(testRes.ok()).toBeTruthy();

  // Allow the async driver call to complete.
  await page.waitForTimeout(500);

  // Assert the mock log gained at least one resolume entry.
  const afterRes = await request.get(mockLogUrl);
  expect(afterRes.ok()).toBeTruthy();
  const afterEntries = (await afterRes.json()) as Array<{
    mock: string;
    method: string;
    path: string;
  }>;
  expect(afterEntries.length).toBeGreaterThan(beforeCount);

  const newEntries = afterEntries.slice(beforeCount);
  const resolumeEntries = newEntries.filter((e) => e.mock === "resolume");
  expect(resolumeEntries.length).toBeGreaterThan(0);

  // Console must be clean.
  expect(consoleMessages).toEqual([]);
});
