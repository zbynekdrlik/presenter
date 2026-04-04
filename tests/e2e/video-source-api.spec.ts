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

test("video source CRUD lifecycle", async ({ request }) => {
  // Create
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Test Camera", ndiName: "CAM1 (usb)" } },
  );
  expect(created.status()).toBe(200);
  const source = await created.json();
  expect(source.label).toBe("Test Camera");
  expect(source.ndiName).toBe("CAM1 (usb)");
  expect(source.isActive).toBe(false);

  // List
  const listed = await request.get(
    new URL("/integrations/video-sources", baseURL).toString(),
  );
  expect(listed.status()).toBe(200);
  const sources = await listed.json();
  expect(sources.length).toBeGreaterThanOrEqual(1);
  expect(sources.some((s: any) => s.id === source.id)).toBe(true);

  // Update
  const updated = await request.put(
    new URL(`/integrations/video-sources/${source.id}`, baseURL).toString(),
    { data: { label: "Main Camera", ndiName: "CAM1 (usb)" } },
  );
  expect(updated.status()).toBe(200);
  const updatedSource = await updated.json();
  expect(updatedSource.label).toBe("Main Camera");

  // Activate
  const activated = await request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
  );
  expect(activated.status()).toBe(200);
  const activatedSource = await activated.json();
  expect(activatedSource.isActive).toBe(true);

  // Deactivate
  const deactivated = await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  expect(deactivated.status()).toBe(200);

  // Delete
  const deleted = await request.delete(
    new URL(`/integrations/video-sources/${source.id}`, baseURL).toString(),
  );
  expect(deleted.status()).toBe(204);

  // Verify deleted
  const afterDelete = await request.get(
    new URL("/integrations/video-sources", baseURL).toString(),
  );
  const remaining = await afterDelete.json();
  expect(remaining.some((s: any) => s.id === source.id)).toBe(false);
});

test("NDI status returns available: false in CI", async ({ request }) => {
  const resp = await request.get(
    new URL("/ndi/status", baseURL).toString(),
  );
  expect(resp.status()).toBe(200);
  const body = await resp.json();
  expect(body.available).toBe(false);
});

test("NDI sources returns 503 when SDK unavailable", async ({ request }) => {
  const resp = await request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  expect(resp.status()).toBe(503);
});
