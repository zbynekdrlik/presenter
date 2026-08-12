/**
 * E2E for #599: the AI panel's PRIMARY logged-out state.
 *
 * Before this, a dead Claude login only surfaced as a small line buried
 * inside the AI panel's collapsed settings drawer — nothing told the
 * operator to use the existing login flow at `/ui/operator/ai` (real
 * incident 2026-07-26: AI verses died right before an event, fixed via SSH
 * instead of the GUI).
 *
 * Mirrors `ai-status-chip.spec.ts`'s technique: the real login state depends
 * on this box's CLIProxyAPI session, so every scenario is driven by
 * intercepting `/ai/status` via `page.route`, never the live proxy.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
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
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function mockAiStatus(
  page: Page,
  proxy: {
    claudeAuthenticated: boolean;
    tokenExpiresAt?: string | null;
    // #679: defaults to `true` (unset) so every EXISTING call site below
    // keeps its original bundled-proxy meaning unchanged.
    requiresClaudeAuth?: boolean;
  },
) {
  const requiresClaudeAuth = proxy.requiresClaudeAuth ?? true;
  const connected = requiresClaudeAuth ? proxy.claudeAuthenticated : true;
  await page.route("**/ai/status", async (route) => {
    await route.fulfill({
      json: {
        connected,
        error: connected
          ? null
          : "Claude not authenticated — run /ai/proxy/login to re-authorize",
        proxy: {
          running: true,
          port: 18787,
          apiUrl: "http://127.0.0.1:18787/v1",
          binaryFound: true,
          claudeAuthenticated: proxy.claudeAuthenticated,
          tokenExpiresAt: proxy.tokenExpiresAt ?? null,
        },
        modelValid: true,
        requiresClaudeAuth,
      },
    });
  });
}

async function gotoAiPanel(page: Page) {
  await page.goto(new URL("/ui/operator/ai", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "ai",
    { timeout: 5_000 },
  );
}

test("an unresolved status check never shows the banner as a false accusation", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  // Malformed 200 body — same technique `ai-status-chip.spec.ts` uses to
  // exercise a failed status fetch without the browser's own non-2xx
  // console noise. `check_status()` resolves `Err`, and never resolves
  // `Ok`, for the lifetime of this test.
  await page.route("**/ai/status", async (route) => {
    await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });

  await gotoAiPanel(page);

  // #622 post-merge review finding 1: before the fix, `proxy_authenticated`
  // defaulted to `false` and a failed fetch never touched it, so the banner
  // painted "Nie si prihlásený" before any real response ever arrived (and
  // would stay that way forever). The real state here is UNKNOWN — the
  // banner must stay hidden, not accuse the operator of being logged out.
  const banner = page.locator('[data-role="ai-login-banner"]');
  await page.waitForTimeout(2_000);
  await expect(banner).toHaveAttribute("data-visible", "false");

  expect(consoleMessages).toEqual([]);
});

test("logged out shows the primary login banner with a visible CTA", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { claudeAuthenticated: false });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  await expect(banner).toHaveAttribute("data-visible", "true", { timeout: 30_000 });
  await expect(banner).toBeVisible();

  const cta = page.locator('[data-role="ai-login-cta"]');
  await expect(cta).toBeVisible();
  await expect(cta).toHaveText("Prihlásiť sa");

  expect(consoleMessages).toEqual([]);
});

test("logged in shows no login banner", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: true,
    tokenExpiresAt: "2099-01-01T00:00:00Z",
  });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  // Always mounted (never conditionally rendered) — assert the attribute,
  // never element count/visibility, per the toast-component discipline.
  await expect(banner).toHaveAttribute("data-visible", "false", { timeout: 30_000 });
  await expect(banner).toBeHidden();

  expect(consoleMessages).toEqual([]);
});

// #679: a user pointing `apiUrl` at their own non-bundled OpenAI-compatible
// endpoint (the #662 local-LLM scenario) never needs a Claude login — the
// login banner must stay hidden AND the header chip must report "ok", even
// though Claude itself is unauthenticated, as long as connectivity to the
// configured endpoint is fine.
test("#679: a non-bundled apiUrl needs no Claude login — banner hidden, chip ok, even with Claude unauthenticated", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: false,
    requiresClaudeAuth: false,
  });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  await expect(banner).toHaveAttribute("data-visible", "false", { timeout: 30_000 });
  await expect(banner).toBeHidden();

  // The header chip is mounted unconditionally alongside every operator
  // view (including the AI panel), so both assertions live in one page load.
  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: pripojené");

  expect(consoleMessages).toEqual([]);
});

test("a known token expiry shows how long the login stays valid while authenticated", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: true,
    tokenExpiresAt: "2099-06-15T12:00:00Z",
  });

  await gotoAiPanel(page);

  const validity = page.locator('[data-role="ai-token-validity"]');
  await expect(validity).toHaveAttribute("data-visible", "true", { timeout: 30_000 });
  await expect(validity).toBeVisible();
  await expect(validity).toHaveText(/plat[íi] do/);
  await expect(validity).toHaveText(/2099/);

  expect(consoleMessages).toEqual([]);
});

test("#660: a token expiring soon shows an explicit renew warning, not the flat valid-until text", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  // Before #660, this note rendered the SAME flat "platí do" line whether 8
  // hours or 8 minutes remained.
  const soon = new Date(Date.now() + 30 * 60 * 1000).toISOString();
  await mockAiStatus(page, {
    claudeAuthenticated: true,
    tokenExpiresAt: soon,
  });

  await gotoAiPanel(page);

  const validity = page.locator('[data-role="ai-token-validity"]');
  await expect(validity).toHaveAttribute("data-visible", "true", { timeout: 30_000 });
  await expect(validity).toHaveText(/čoskoro vyprší/);
  await expect(validity).toHaveText(/odporúčame sa znova prihlásiť/);

  expect(consoleMessages).toEqual([]);
});

test("an expired token is named in the logged-out banner's subtext", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: false,
    tokenExpiresAt: "2020-03-10T08:00:00Z",
  });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  await expect(banner).toHaveAttribute("data-visible", "true", { timeout: 30_000 });

  const subtext = banner.locator(".ai-chat__login-banner-subtext");
  await expect(subtext).toHaveText(/vypr[šs]alo/);
  await expect(subtext).toHaveText(/2020/);

  // The validity note is the "still valid" state — must stay hidden while
  // logged out, even though we DO know an expiry.
  const validity = page.locator('[data-role="ai-token-validity"]');
  await expect(validity).toHaveAttribute("data-visible", "false");

  expect(consoleMessages).toEqual([]);
});

test("clicking the CTA starts the existing Claude login flow", async ({
  page,
}) => {
  // #622 post-merge review finding 7: the CTA reuses `pages/ai.rs`'s
  // existing `proxy_login()` flow (never a duplicate) — prove the click
  // actually fires POST /ai/proxy/login, the same request the settings-drawer
  // "Claude Login" button uses.
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { claudeAuthenticated: false });

  let loginRequested = false;
  await page.route("**/ai/proxy/login", async (route) => {
    loginRequested = true;
    await route.fulfill({
      json: { loginUrl: "https://claude.ai/oauth/authorize?fake=1" },
    });
  });

  await gotoAiPanel(page);

  const cta = page.locator('[data-role="ai-login-cta"]');
  await expect(cta).toBeVisible();
  await cta.click();

  await expect.poll(() => loginRequested).toBe(true);

  expect(consoleMessages).toEqual([]);
});

test("a chat auth error points the operator at the visible login banner", async ({
  page,
}) => {
  // #622 post-merge review finding 6: before the fix, a chat-time auth
  // failure only re-checked `/ai/status` silently — the displayed error text
  // never told the operator WHERE to go. Mock an SSE "error" event from
  // /ai/chat while /ai/status reports logged-out, and assert the error text
  // carries the login hint (banner is already visible from the status mock).
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { claudeAuthenticated: false });

  await page.route("**/ai/chat", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "text/event-stream",
      body: 'event: error\ndata: {"message":"authentication_error"}\n\n',
    });
  });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  await expect(banner).toHaveAttribute("data-visible", "true", { timeout: 30_000 });

  const textarea = page.locator('[data-role="ai-input"]');
  await textarea.fill("ahoj");
  await page.locator('[data-role="ai-send"]').click();

  const error = page.locator('[data-role="ai-error"]');
  await expect(error).toBeVisible({ timeout: 15_000 });
  await expect(error).toContainText("authentication_error");
  await expect(error).toContainText(/prihlásenie ku Claude vypršalo/);

  expect(consoleMessages).toEqual([]);
});
