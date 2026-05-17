import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 120_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("toggling ableset settings via HTTP creates an http_setter audit row", async ({
  request,
}) => {
  const beforeResp = await request.get(
    new URL(
      "/integrations/audit?table=ableset_settings&limit=100",
      baseURL,
    ).toString(),
  );
  expect(beforeResp.ok()).toBeTruthy();
  const beforeRows = (await beforeResp.json()) as Array<{ source: string }>;
  const beforeHttp = beforeRows.filter(
    (r) => r.source === "http_setter",
  ).length;

  const currentResp = await request.get(
    new URL("/integrations/ableset/settings", baseURL).toString(),
  );
  expect(currentResp.ok()).toBeTruthy();
  const current = await currentResp.json();
  const updated = { ...current, enabled: !current.enabled };

  const putResp = await request.put(
    new URL("/integrations/ableset/settings", baseURL).toString(),
    { data: updated },
  );
  expect(putResp.ok()).toBeTruthy();

  const afterResp = await request.get(
    new URL(
      "/integrations/audit?table=ableset_settings&limit=100",
      baseURL,
    ).toString(),
  );
  expect(afterResp.ok()).toBeTruthy();
  const afterRows = (await afterResp.json()) as Array<{
    source: string;
    afterJson: { enabled: boolean };
  }>;
  const afterHttp = afterRows.filter((r) => r.source === "http_setter").length;
  expect(afterHttp).toBe(beforeHttp + 1);
  expect(afterRows[0].afterJson.enabled).toBe(updated.enabled);

  // Restore original settings.
  await request.put(
    new URL("/integrations/ableset/settings", baseURL).toString(),
    { data: current },
  );
});
