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

// #850 S8: on LAN http (HTTP/1.1, six connections per host) fifteen stylesheets
// took three rounds before the wasm could start. One file, in the source order:
// a later file's rule must still beat an earlier one's.
test("a cold load fetches one stylesheet, with every source file in order", async ({
  browser,
}) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const css: string[] = [];
  page.on("request", (req) => {
    const path = new URL(req.url()).pathname;
    if (path.startsWith("/ui-pkg/") && path.endsWith(".css")) css.push(path);
  });
  try {
    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    expect(css).toHaveLength(1);

    const order = await page.evaluate(() => {
      const sheet = [...document.styleSheets].find((s) => s.href?.includes("/ui-pkg/"));
      const selectors = [...(sheet?.cssRules ?? [])].map((r) => (r as CSSStyleRule).selectorText ?? "");
      const at = (needle: string) => selectors.findIndex((s) => s.includes(needle));
      // operator.css is the first source file; the data-shell rules come from later ones.
      return { operator: at(".operator__main"), phone: at("data-shell") };
    });
    expect(order.operator).toBeGreaterThanOrEqual(0);
    expect(order.phone).toBeGreaterThan(order.operator);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
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
