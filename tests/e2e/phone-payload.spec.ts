/**
 * #850 B1c — what a phone downloads (spec §6.6, S9): one .wasm per cold load,
 * sent precompressed as brotli (Chrome) or gzip, never the stage-preview app a
 * second time. The negotiation itself is unit-tested in `router/wasm_ui.rs`.
 */
import { test, expect } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("a phone's cold load fetches exactly one wasm, as brotli, and no second app", async ({
  browser,
  request,
}) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const wasm: Array<{ url: string; encoding: string | undefined }> = [];
  const previews: string[] = [];
  page.on("response", (res) => {
    if (new URL(res.url()).pathname.endsWith(".wasm")) {
      wasm.push({ url: res.url(), encoding: res.headers()["content-encoding"] });
    }
  });
  page.on("request", (req) => {
    if (req.url().includes("/stage?preview=1")) previews.push(req.url());
  });
  try {
    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="console-field"]')).toBeVisible();
    await page.locator('[data-role="phone-tab"][data-view="worship"]').tap();
    await expect(page.locator("body")).toHaveAttribute("data-view", "worship");
    expect(wasm).toHaveLength(1);
    expect(wasm[0].encoding).toBe("br");
    expect(previews).toEqual([]);

    // Older clients: gzip, or the file itself; every answer varies on the header.
    const gz = await request.get(wasm[0].url, { headers: { "Accept-Encoding": "gzip" } });
    expect(gz.headers()["content-encoding"]).toBe("gzip");
    expect(gz.headers()["vary"]).toBe("accept-encoding");
    const plain = await request.get(wasm[0].url, { headers: { "Accept-Encoding": "identity" } });
    expect(plain.headers()["content-encoding"]).toBeUndefined();
    expect(plain.headers()["content-type"]).toBe("application/wasm");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
