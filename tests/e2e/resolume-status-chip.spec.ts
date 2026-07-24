/**
 * E2E for #564: the operator-header Resolume connection chip.
 *
 * - With no enabled host, the chip area must render absent — zero chips,
 *   zero console noise. This is the ONLY scenario exercised on the real dev
 *   instance (no Resolume is configured there).
 * - With an enabled host reachable through the embedded Node mock
 *   (`startMockResolume`, an arbitrary free port — never the fixed
 *   127.0.0.1:8091 the Rust mock-integrations listener owns), the chip must
 *   show green while connected and yellow the moment the mock goes offline.
 *
 * Full port-drift adoption/heal-back is proven by the Rust integration
 * tests (`resolume/port_drift_integration_tests.rs`) against a mock Arena
 * that moves ports mid-test — this spec does not attempt that here.
 */

import { test, expect } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startMockResolume,
  startTestServer,
  stopServer,
  type MockResolumeHandle,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let mockResolume: MockResolumeHandle | undefined;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  mockResolume = await startMockResolume();
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  if (mockResolume) {
    await mockResolume.close();
    mockResolume = undefined;
  }
});

test("no resolume host configured — the chip area renders absent, console clean", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  // The container always mounts (data-role="resolume-status-chips"); with
  // no enabled hosts it must hold zero chips — never a placeholder, never a
  // console error.
  await expect(page.locator('[data-role="resolume-status-chips"]')).toBeAttached();
  await expect(page.locator('[data-role="resolume-status-chip"]')).toHaveCount(0);

  // #573: the chips live in the TOP brand/surface-nav row — never next to
  // the Stage Output select in `operator__header-right`.
  await expect(
    page.locator('.operator__header-brand [data-role="resolume-status-chips"]'),
  ).toHaveCount(1);
  await expect(
    page.locator('.operator__header-right [data-role="resolume-status-chips"]'),
  ).toHaveCount(0);

  expect(consoleMessages).toEqual([]);
});

test("an enabled host tracks the mock's connection state: green while up, yellow once it goes offline", async ({
  page,
  request,
}) => {
  if (!mockResolume) {
    throw new Error("Mock Resolume server not started");
  }
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);

  const label = "E2E chip mock";
  const createRes = await request.post(
    new URL("/integrations/resolume/hosts", baseURL).toString(),
    {
      data: {
        label,
        host: "127.0.0.1",
        port: mockResolume.port,
        isEnabled: true,
      },
    },
  );
  expect(createRes.ok()).toBeTruthy();
  const created = (await createRes.json()) as { id: string };

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator(
    `[data-role="resolume-status-chip"][data-host-label="${label}"]`,
  );
  await expect(chip).toHaveAttribute("data-state", "green", { timeout: 30_000 });
  // The visible chip text is the HOST'S NAME — with several walls
  // configured, a bare "Connected" never says WHICH Resolume it is.
  // Connection state lives in the dot color + tooltip.
  await expect(chip.locator(".operator__resolume-chip-label")).toHaveText(label);
  await expect(chip).toHaveAttribute("title", /Connected/);

  // Simulate the mock going offline (Arena crashing / unreachable) — the
  // chip must flip to yellow while KEEPING the host's name as its text,
  // and its tooltip must name a concrete cause.
  mockResolume.setOnline(false);
  await expect(chip).toHaveAttribute("data-state", "yellow", { timeout: 30_000 });
  await expect(chip.locator(".operator__resolume-chip-label")).toHaveText(label);
  await expect(chip).toHaveAttribute("title", /Not reachable/);

  // Recovery: back online, the chip must return to green on its own.
  mockResolume.setOnline(true);
  await expect(chip).toHaveAttribute("data-state", "green", { timeout: 30_000 });

  await request.delete(
    new URL(`/integrations/resolume/hosts/${created.id}`, baseURL).toString(),
  );

  expect(consoleMessages).toEqual([]);
});
