/**
 * #915 ROZHODNUTÉ 1 — "zapamätané zariadenie": a bookmark saved at home (logged
 * in through Cloudflare Access) is on the same phone's list at church, where the
 * Access bypass sends no login. The server verifies the Access token (RS256, the
 * site's AUD, the team issuer) against a mock of the team's key URL, hands the
 * browser a signed device cookie, and later knows the person from that cookie
 * alone. A browser with neither sees the shared list. Nothing here reaches a
 * wall (T11).
 *
 * The key pairs are generated for this run (none is committed); no deployed
 * server trusts them.
 */
import { test, expect, type BrowserContextOptions, type Locator, type Page } from "@playwright/test";
import { createSign, generateKeyPairSync, type KeyObject } from "node:crypto";
import { mkdtempSync } from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import { deriveTestConfig, PHONE_390, refreshDevData, startTestServer, stopServer, type ServerHandle } from "./support";
import { finish, openStudy, verseNumber, type OpenedStudy } from "./study-support";

test.describe.configure({ timeout: 180_000 });

const KID = "test-signing";
const newKey = (): KeyObject => generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey;
/** The key the mock team publishes, and a key nobody trusts (same key id). */
const SIGNING = newKey();
const FORGER = newKey();
const TEAM = "team.example.cloudflareaccess.com";
const AUD = "e2eaudtag915";
const ACCESS_ENV = {
  PRESENTER_ACCESS_TEAM_DOMAIN: TEAM,
  PRESENTER_ACCESS_AUD: AUD,
} as const;

let serverHandle: ServerHandle | undefined;
let certsServer: http.Server | undefined;
let baseURL = "";

const base64url = (data: string | Buffer): string => Buffer.from(data).toString("base64url");

/** A token as Cloudflare Access signs it for this site, valid for an hour. */
function accessToken(email: string, key: KeyObject = SIGNING): string {
  const now = Math.floor(Date.now() / 1000);
  const header = base64url(JSON.stringify({ alg: "RS256", kid: KID, typ: "JWT" }));
  const claims = base64url(
    JSON.stringify({ aud: [AUD], email, exp: now + 3600, iat: now, nbf: now, iss: `https://${TEAM}`, type: "app" }),
  );
  const signer = createSign("RSA-SHA256");
  signer.update(`${header}.${claims}`);
  const signature = signer.sign(key).toString("base64url");
  return `${header}.${claims}.${signature}`;
}

test.beforeAll(async ({}, testInfo) => {
  const jwk = SIGNING.export({ format: "jwk" });
  const certs = JSON.stringify({ keys: [{ kid: KID, kty: "RSA", alg: "RS256", use: "sig", n: jwk.n, e: jwk.e }] });
  const mock = http.createServer((req, res) => {
    if (req.method === "GET" && req.url === "/cdn-cgi/access/certs") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(certs);
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });
  certsServer = mock;
  await new Promise<void>((resolve) => mock.listen(0, "127.0.0.1", () => resolve()));
  const mockPort = (mock.address() as AddressInfo).port;

  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  const overrides: Record<string, string> = {
    ...ACCESS_ENV,
    PRESENTER_ACCESS_CERTS_URL: `http://127.0.0.1:${mockPort}/cdn-cgi/access/certs`,
    PRESENTER_ACCESS_DEVICE_KEY_FILE: path.join(mkdtempSync(path.join(os.tmpdir(), "presenter-915-")), "device.key"),
  };
  const previous = Object.fromEntries(Object.keys(overrides).map((name) => [name, process.env[name]]));
  Object.assign(process.env, overrides);
  try {
    serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
  } finally {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  await new Promise<void>((resolve) => (certsServer ? certsServer.close(() => resolve()) : resolve()));
});

const sheet = (page: Page): Locator => page.locator('[data-role="bookmarks"]');
const bookmark = (page: Page, name: string): Locator =>
  sheet(page)
    .locator('[data-role="bookmark-row"]')
    .filter({ has: page.locator('[data-role="bookmark-open"] .study-row__label', { hasText: name }) });

async function openList(page: Page): Promise<void> {
  await page.locator('[data-role="study-bookmarks"]').tap();
  await expect(page).toHaveURL(/s=zalozky/);
  await expect(sheet(page)).toHaveAttribute("data-state", "ready");
}

async function identity(page: Page): Promise<unknown> {
  return page.evaluate(async () => (await fetch("/bible/study/identity")).json());
}

test("a bookmark saved at home is on the same phone at church, and nobody else's list", async ({ browser }) => {
  // #932: one tap adds it under the verse's place.
  const name = "Hebrejom 11:3";

  // At home: Cloudflare Access logged the preacher in and sends its token.
  const home = await openStudy(browser, baseURL, {
    ...PHONE_390,
    extraHTTPHeaders: { "cf-access-jwt-assertion": accessToken("Preacher@Example.org") },
  } as BrowserContextOptions);
  expect(await identity(home.page)).toEqual({ email: "preacher@example.org" });
  await verseNumber(home.page, 3).tap();
  await home.page.locator('[data-role="verse-save-bookmark"]').tap();
  await expect(home.page.locator('[data-role="study-notice"]')).toContainText(`Záložka pridaná: ${name}`);

  // The server remembered this phone: an httpOnly cookie, not readable by the page.
  const cookies = await home.context.cookies(baseURL);
  const device = cookies.find((cookie) => cookie.name === "presenter_device");
  expect(device, "the device cookie").toBeDefined();
  expect(device?.httpOnly).toBe(true);
  expect(device?.sameSite).toBe("Lax");
  expect(await home.page.evaluate(() => document.cookie)).not.toContain("presenter_device");
  await finish(home);

  // At church: the bypass sends no token, the same phone still has its cookie.
  const church = await openStudy(browser, baseURL, PHONE_390, undefined, undefined, async (page) => {
    await page.context().addCookies(cookies.filter((cookie) => cookie.name === "presenter_device"));
  });
  expect(await identity(church.page)).toEqual({ email: "preacher@example.org" });
  await openList(church.page);
  await expect(bookmark(church.page, name).locator(".study-row__detail")).toHaveText("Hebrejom 11:3 (SEB)");

  // Another phone, never logged in (or the LAN address): the shared list.
  const other: OpenedStudy = await openStudy(browser, baseURL, PHONE_390);
  expect(await identity(other.page)).toEqual({ email: null });
  await openList(other.page);
  await expect(bookmark(other.page, name)).toHaveCount(0);
  // A token signed by someone else's key names nobody either.
  const forged = await other.page.evaluate(
    async (token) => (await fetch("/bible/study/identity", { headers: { "cf-access-jwt-assertion": token } })).json(),
    accessToken("preacher@example.org", FORGER),
  );
  expect(forged).toEqual({ email: null });
  await finish(other);

  // Clean up at church: the preacher deletes it from his own list.
  await bookmark(church.page, name).locator('[data-role="bookmark-delete"]').tap();
  await expect(bookmark(church.page, name)).toHaveCount(0);
  await expect(church.page.locator('[data-role="study-notice"]')).toContainText(`Zmazané: ${name}`);
  await finish(church);
});
