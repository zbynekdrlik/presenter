/**
 * WASM AI Chat Tab Tests
 *
 * Tests the AI chat tab UI: navigation, settings panel, message input,
 * error handling, and conversation clearing.
 *
 * Note: These tests do NOT test actual AI responses (which require a
 * reachable, credentialed AI backend). They verify the UI behavior, API
 * integration plumbing, and error handling.
 */

import { test, expect } from "@playwright/test";
import http from "http";
import type { AddressInfo } from "net";
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
});

async function initPage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

async function navigateToAi(page: import("@playwright/test").Page) {
  await initPage(page);
  const aiButton = page.locator('[data-role="view-toggle"][data-view="ai"]');
  await aiButton.click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "ai",
    { timeout: 5_000 },
  );
}

test.describe("AI Tab Navigation", () => {
  test("AI tab button exists in header navigation", async ({ page }) => {
    await initPage(page);

    const aiButton = page.locator('[data-role="view-toggle"][data-view="ai"]');
    await expect(aiButton).toBeVisible();
    await expect(aiButton).toHaveText("AI");
  });

  test("clicking AI tab changes data-view to ai", async ({ page }) => {
    await navigateToAi(page);

    const body = page.locator("body");
    const view = await body.getAttribute("data-view");
    expect(view).toBe("ai");
  });

  test("AI panel is visible when AI view is active", async ({ page }) => {
    await navigateToAi(page);

    const aiPanel = page.locator('[data-view-panel="ai"]');
    await expect(aiPanel).toBeVisible();
  });

  test("AI chat container renders with correct structure", async ({ page }) => {
    await navigateToAi(page);

    const chat = page.locator('[data-role="ai-chat"]');
    await expect(chat).toBeVisible();

    // Header with title
    const title = chat.locator("h2");
    await expect(title).toHaveText("AI Assistant");

    // Settings and Clear buttons
    const settingsBtn = page.locator('[data-role="ai-settings-toggle"]');
    await expect(settingsBtn).toBeVisible();

    const clearBtn = page.locator('[data-role="ai-clear"]');
    await expect(clearBtn).toBeVisible();
  });

  test("direct navigation to /ui/operator/ai opens AI view", async ({
    page,
  }) => {
    await page.goto(`${baseURL}/ui/operator/ai`);
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });

    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "ai",
      { timeout: 5_000 },
    );
    const url = new URL(page.url());
    expect(url.pathname).toBe("/ui/operator/ai");
  });
});

test.describe("AI Chat Empty State", () => {
  test("shows empty state message when no messages", async ({ page }) => {
    await navigateToAi(page);

    const messages = page.locator('[data-role="ai-messages"]');
    await expect(messages).toBeVisible();

    // Should show helper text
    await expect(
      messages.getByText("Paste the pastor's message"),
    ).toBeVisible();
    await expect(messages.getByText("Bible references")).toBeVisible();
  });

  test("textarea and Send button are present", async ({ page }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    await expect(textarea).toBeVisible();
    await expect(textarea).toHaveAttribute(
      "placeholder",
      "Type a message or paste pastor's text...",
    );

    const sendBtn = page.locator('[data-role="ai-send"]');
    await expect(sendBtn).toBeVisible();
    await expect(sendBtn).toBeDisabled(); // Disabled when empty
  });
});

test.describe("AI Chat Input Behavior", () => {
  test("Send button enables when text is entered", async ({ page }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    const sendBtn = page.locator('[data-role="ai-send"]');

    await expect(sendBtn).toBeDisabled();
    await textarea.fill("test message");
    await expect(sendBtn).toBeEnabled();
  });

  test("Send button stays disabled for whitespace-only input", async ({
    page,
  }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    const sendBtn = page.locator('[data-role="ai-send"]');

    await textarea.fill("   ");
    await expect(sendBtn).toBeDisabled();
  });

  test("sending a message shows user message bubble", async ({ page }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.fill("Hello AI");

    const sendBtn = page.locator('[data-role="ai-send"]');
    await sendBtn.click();

    // User message should appear
    const userMsg = page.locator(
      '[data-role="ai-message"][data-message-role="user"]',
    );
    await expect(userMsg).toBeVisible({ timeout: 5_000 });
    await expect(userMsg).toContainText("Hello AI");

    // Textarea should be cleared after send
    await expect(textarea).toHaveValue("");
  });

  test("sending shows error when AI is not configured", async ({ page }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.fill("test");

    const sendBtn = page.locator('[data-role="ai-send"]');
    await sendBtn.click();

    // #762 CI follow-up: startTestServer points the AI backend at a DEAD
    // loopback endpoint (http://127.0.0.1:1/v1) with NO key — a deterministic,
    // third-party-free setup. A loopback host is exempt from the keyless
    // preflight guard, so the request is actually attempted and fails to
    // connect, surfacing the exact "failed to reach AI API" message. Assert it
    // precisely — the old broad regex also matched a live-backend 401, hiding a
    // real egress-to-openrouter.ai failure.
    const error = page.locator('[data-role="ai-error"]');
    await expect(error).toBeVisible({ timeout: 15_000 });
    await expect(error).toContainText(/failed to reach AI API/);
  });
});

test.describe("AI Settings Panel", () => {
  test("settings panel is hidden by default", async ({ page }) => {
    await navigateToAi(page);

    const settingsPanel = page.locator('[data-role="ai-settings-panel"]');
    await expect(settingsPanel).toBeHidden();
  });

  test("clicking gear icon toggles settings panel", async ({ page }) => {
    await navigateToAi(page);

    const settingsToggle = page.locator('[data-role="ai-settings-toggle"]');
    const settingsPanel = page.locator('[data-role="ai-settings-panel"]');

    // Open
    await settingsToggle.click();
    await expect(settingsPanel).toBeVisible();

    // Close
    await settingsToggle.click();
    await expect(settingsPanel).toBeHidden();
  });

  test("settings panel has API URL, API Key, and Model fields", async ({
    page,
  }) => {
    await navigateToAi(page);

    const settingsToggle = page.locator('[data-role="ai-settings-toggle"]');
    await settingsToggle.click();

    const apiUrl = page.locator('[data-role="ai-api-url"]');
    await expect(apiUrl).toBeVisible();

    const apiKey = page.locator('[data-role="ai-api-key"]');
    await expect(apiKey).toBeVisible();

    const model = page.locator('[data-role="ai-model"]');
    await expect(model).toBeVisible();

    const saveBtn = page.locator('[data-role="ai-save-settings"]');
    await expect(saveBtn).toBeVisible();
  });

  // #677: Chrome logs a VERBOSE `[DOM] Password field is not contained in a
  // form` console hint for the API Key field, which has no <form> ancestor.
  // VERBOSE is neither `error` nor `warning`, so the standard zero-console
  // collector below can NOT catch a regression here — the DOM-relationship
  // assertion is what actually pins the fix. The Enter-key check pins the
  // "no native submit / no page reload" behavior-preservation constraint:
  // a bare <form> with no explicit submit handling would GET-reload the
  // whole SPA (losing all WASM state) the first time a user pressed Enter
  // in one of these fields.
  test("API Key field has a form ancestor, and Enter does not reload the page", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);

    await navigateToAi(page);

    const settingsToggle = page.locator('[data-role="ai-settings-toggle"]');
    await settingsToggle.click();

    const apiKey = page.locator('[data-role="ai-api-key"]');
    await expect(apiKey).toBeVisible();

    const hasFormAncestor = await apiKey.evaluate(
      (el) => el.closest("form") !== null,
    );
    expect(hasFormAncestor).toBe(true);

    const urlBefore = page.url();
    await apiKey.click();
    await apiKey.press("Enter");

    // A native submit with no handler would GET-reload the current URL —
    // assert it never does: same URL, and the settings panel (which a
    // reload would reset to its default-closed state) is still open.
    expect(page.url()).toBe(urlBefore);
    await expect(
      page.locator('[data-role="ai-settings-panel"]'),
    ).toBeVisible();

    expect(consoleMessages).toEqual([]);
  });

  test("settings fields are pre-populated from server", async ({ page }) => {
    await navigateToAi(page);

    const settingsToggle = page.locator('[data-role="ai-settings-toggle"]');
    await settingsToggle.click();

    // Model should have a default value
    const model = page.locator('[data-role="ai-model"]');
    const modelValue = await model.inputValue();
    expect(modelValue.length).toBeGreaterThan(0);
  });

  test("saving settings calls API and shows toast", async ({ page }) => {
    await navigateToAi(page);

    const settingsToggle = page.locator('[data-role="ai-settings-toggle"]');
    await settingsToggle.click();

    // Change model to a test value
    const model = page.locator('[data-role="ai-model"]');
    await model.fill("test-model-123");

    const saveBtn = page.locator('[data-role="ai-save-settings"]');
    await saveBtn.click();

    // Should show success toast
    await expect(page.getByText("AI settings saved")).toBeVisible({
      timeout: 5_000,
    });

    // Reload and verify persistence
    await page.reload();
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "ai",
      { timeout: 5_000 },
    );

    await page.locator('[data-role="ai-settings-toggle"]').click();
    const modelAfter = page.locator('[data-role="ai-model"]');
    await expect(modelAfter).toHaveValue("test-model-123");

    // Restore default
    await modelAfter.fill("claude-sonnet-4-20250514");
    await page.locator('[data-role="ai-save-settings"]').click();
  });
});

test.describe("AI Conversation Management", () => {
  test("Clear button removes messages and errors", async ({ page }) => {
    // Reset persisted conversation before the page loads so the on-mount
    // restore in ai.rs cannot resurrect messages from a prior test (#247).
    await page.request.post(`${baseURL}/ai/clear`, { data: {} });

    await navigateToAi(page);

    // Send a message (will produce an error in test environment)
    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.fill("test clear");
    await page.locator('[data-role="ai-send"]').click();

    // Wait for exactly one user message to appear (sent message above).
    const userMsg = page.locator(
      '[data-role="ai-message"][data-message-role="user"]',
    );
    await expect(userMsg).toHaveCount(1, { timeout: 5_000 });
    await expect(userMsg).toBeVisible({ timeout: 5_000 });

    // Click Clear
    await page.locator('[data-role="ai-clear"]').click();

    // Messages should be gone, empty state should return
    await expect(userMsg).not.toBeVisible();
    await expect(page.getByText("Paste the pastor's message")).toBeVisible();

    // Error should also be cleared
    const error = page.locator('[data-role="ai-error"]');
    await expect(error).not.toBeVisible();
  });
});

test.describe("AI Chat Connection Status", () => {
  test("connection status indicator is visible", async ({ page }) => {
    await navigateToAi(page);

    const statusDot = page.locator('[data-role="ai-chat"] .ai-chat__status');
    await expect(statusDot).toBeVisible();
  });

  test("status endpoint returns the backend-agnostic shape", async ({ page }) => {
    // Direct API test — since #762 the payload is `{connected, error, modelValid}`
    // with no nested `proxy` object and no `requiresClaudeAuth` flag.
    const response = await page.request.get(`${baseURL}/ai/status`);
    expect(response.ok()).toBe(true);

    const data = await response.json();
    expect(data).toHaveProperty("connected");
    expect(data).toHaveProperty("error");
    expect(data).toHaveProperty("modelValid");
    expect(data).not.toHaveProperty("proxy");
    expect(data).not.toHaveProperty("requiresClaudeAuth");
  });
});

test.describe("AI Chat API Endpoints", () => {
  test("GET /ai/settings returns settings", async ({ page }) => {
    const response = await page.request.get(`${baseURL}/ai/settings`);
    expect(response.ok()).toBe(true);

    const data = await response.json();
    expect(data).toHaveProperty("apiUrl");
    expect(data).toHaveProperty("apiKeySet");
    expect(data).toHaveProperty("model");
  });

  test("PUT /ai/settings updates settings", async ({ page }) => {
    const response = await page.request.put(`${baseURL}/ai/settings`, {
      data: { model: "test-e2e-model" },
    });
    expect(response.status()).toBe(204);

    // Verify
    const getResponse = await page.request.get(`${baseURL}/ai/settings`);
    const data = await getResponse.json();
    expect(data.model).toBe("test-e2e-model");

    // Restore
    await page.request.put(`${baseURL}/ai/settings`, {
      data: { model: "claude-sonnet-4-20250514" },
    });
  });

  test("POST /ai/clear returns 204", async ({ page }) => {
    const response = await page.request.post(`${baseURL}/ai/clear`, {
      data: {},
    });
    expect(response.status()).toBe(204);
  });

  test("POST /ai/chat with empty message returns 400", async ({ page }) => {
    const response = await page.request.post(`${baseURL}/ai/chat`, {
      data: { message: "" },
    });
    expect(response.status()).toBe(400);
  });

  test("POST /ai/chat with whitespace returns 400", async ({ page }) => {
    const response = await page.request.post(`${baseURL}/ai/chat`, {
      data: { message: "   " },
    });
    expect(response.status()).toBe(400);
  });
});

test.describe("AI Chat Layout", () => {
  test("AI panel uses full height", async ({ page }) => {
    await navigateToAi(page);

    const chat = page.locator('[data-role="ai-chat"]');
    const box = await chat.boundingBox();
    expect(box).not.toBeNull();
    // Chat should occupy meaningful vertical space
    expect(box!.height).toBeGreaterThan(200);
  });

  test("user message is right-aligned", async ({ page }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.fill("alignment test");
    await page.locator('[data-role="ai-send"]').click();

    const userMsg = page.locator(
      '[data-role="ai-message"][data-message-role="user"]',
    );
    await expect(userMsg).toBeVisible({ timeout: 5_000 });

    // Check it has the right-aligned class via computed style
    const alignSelf = await userMsg.evaluate(
      (el) => window.getComputedStyle(el).alignSelf,
    );
    expect(alignSelf).toBe("flex-end");
  });
});

test.describe("AI Chat Paste Handler", () => {
  test("pasting HTML with bold tags converts to ## markers", async ({
    page,
  }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.focus();

    // Dispatch paste event with HTML containing <b> and <strong> tags
    await page.evaluate(() => {
      const ta = document.querySelector(
        '[data-role="ai-input"]',
      ) as HTMLTextAreaElement;
      const dt = new DataTransfer();
      dt.setData(
        "text/html",
        "Normal text <b>bold text</b> and <strong>strong text</strong> end",
      );
      dt.setData("text/plain", "Normal text bold text and strong text end");
      const event = new ClipboardEvent("paste", {
        clipboardData: dt,
        bubbles: true,
        cancelable: true,
      });
      ta.dispatchEvent(event);
    });

    // Verify the textarea contains ## markers around bold text
    const value = await textarea.inputValue();
    expect(value).toContain("##bold text##");
    expect(value).toContain("##strong text##");
    expect(value).toContain("Normal text");
    expect(value).toContain("end");
    // Non-bold text should NOT have ## markers
    expect(value).not.toMatch(/##Normal text##/);
    expect(value).not.toMatch(/##end##/);
  });

  test("pasting HTML with font-weight:bold style converts to ## markers", async ({
    page,
  }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.focus();

    // Dispatch paste event with CSS font-weight:bold style
    await page.evaluate(() => {
      const ta = document.querySelector(
        '[data-role="ai-input"]',
      ) as HTMLTextAreaElement;
      const dt = new DataTransfer();
      dt.setData(
        "text/html",
        'Before <span style="font-weight:bold">styled bold</span> after',
      );
      dt.setData("text/plain", "Before styled bold after");
      const event = new ClipboardEvent("paste", {
        clipboardData: dt,
        bubbles: true,
        cancelable: true,
      });
      ta.dispatchEvent(event);
    });

    const value = await textarea.inputValue();
    expect(value).toContain("##styled bold##");
    expect(value).toContain("Before");
    expect(value).toContain("after");
  });

  test("pasting plain text without HTML does not add markers", async ({
    page,
  }) => {
    await navigateToAi(page);

    const textarea = page.locator('[data-role="ai-input"]');
    // Pre-fill to verify handler doesn't modify existing text
    await textarea.fill("existing text");
    await textarea.focus();

    // Dispatch paste with ONLY text/plain (no text/html)
    const wasDefaultPrevented = await page.evaluate(() => {
      const ta = document.querySelector(
        '[data-role="ai-input"]',
      ) as HTMLTextAreaElement;
      const dt = new DataTransfer();
      dt.setData("text/plain", "just plain text");
      // No text/html — handler should return early without preventing default
      const event = new ClipboardEvent("paste", {
        clipboardData: dt,
        bubbles: true,
        cancelable: true,
      });
      ta.dispatchEvent(event);
      return event.defaultPrevented;
    });

    // Handler should NOT prevent default for plain text paste
    expect(wasDefaultPrevented).toBe(false);

    // Textarea should not contain ## markers
    const value = await textarea.inputValue();
    expect(value).not.toContain("##");
  });
});

test.describe("AI Chat Give-Up (agent guard #784)", () => {
  // The operator asks for a passage; a mock AI provider keeps answering with a
  // tool call the validator rejects (raw ## markers → unprocessed_bold_markers).
  // The #784 agent guard must END the turn after 3 identical rejections with the
  // Slovak give-up message shown as a normal assistant bubble — never a spin to
  // the 120 s client-timeout error the PP operator hit (2026-09-20). Needs its
  // OWN server + isolated DB so the mock-backed AI URL never leaks into the
  // shared server (whose default is a dead loopback endpoint).
  let mockServer: http.Server | undefined;
  let giveUpServer: ServerHandle | undefined;
  let giveUpBaseURL: string;

  test.beforeAll(async ({}, testInfo) => {
    let calls = 0;
    mockServer = http.createServer((req, res) => {
      let body = "";
      req.on("data", (chunk) => {
        body += chunk;
      });
      req.on("end", () => {
        const url = req.url ?? "";
        if (req.method === "GET" && url.includes("/models")) {
          res.writeHead(200, { "content-type": "application/json" });
          res.end(
            JSON.stringify({
              data: [{ id: "google/gemini-3.8-flash" }, { id: "test-model" }],
            }),
          );
          return;
        }
        if (req.method === "POST" && url.includes("/chat/completions")) {
          calls += 1;
          res.writeHead(200, { "content-type": "application/json" });
          res.end(
            JSON.stringify({
              choices: [
                {
                  message: {
                    role: "assistant",
                    content: null,
                    tool_calls: [
                      {
                        id: `call_${calls}`,
                        type: "function",
                        function: {
                          name: "create_bible_presentation",
                          arguments: JSON.stringify({
                            name: "Loop",
                            items: [
                              {
                                kind: "verse",
                                number: 1,
                                text: "##bad## text",
                                book: "Ján",
                                chapter: 1,
                                translation: "SEB",
                              },
                            ],
                          }),
                        },
                      },
                    ],
                  },
                  finish_reason: "tool_calls",
                },
              ],
            }),
          );
          return;
        }
        res.writeHead(404);
        res.end("not found");
      });
    });
    await new Promise<void>((resolve) => {
      mockServer!.listen(0, "127.0.0.1", () => resolve());
    });
    const mockPort = (mockServer!.address() as AddressInfo).port;

    const config = deriveTestConfig(testInfo);
    // +50 stays inside THIS worker's 100-port block (file offsets are 0-49), so
    // it never collides with the shared server or another file's server.
    const port = config.port + 50;
    giveUpBaseURL = `http://127.0.0.1:${port}`;
    const dbUrl = config.dbUrl.replace(/\.db$/, "_giveup.db");
    await refreshDevData(dbUrl);

    // The mock-integrations build binds mock-resolume on a FIXED port (8091),
    // so two test servers cannot coexist: stop the file-level shared server
    // first (this is the last describe in the file; nothing else needs it).
    await stopServer(serverHandle);
    serverHandle = undefined;

    const prev = process.env.PRESENTER_AI_API_URL;
    process.env.PRESENTER_AI_API_URL = `http://127.0.0.1:${mockPort}`;
    try {
      giveUpServer = await startTestServer(port, dbUrl);
    } finally {
      if (prev === undefined) delete process.env.PRESENTER_AI_API_URL;
      else process.env.PRESENTER_AI_API_URL = prev;
    }
  });

  test.afterAll(async () => {
    await stopServer(giveUpServer);
    if (mockServer) {
      await new Promise<void>((resolve) => mockServer!.close(() => resolve()));
    }
  });

  test("operator sees the Slovak give-up message, console stays clean", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);

    await page.goto(`${giveUpBaseURL}/ui/operator/ai`);
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "ai",
      { timeout: 5_000 },
    );

    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.fill("sprav prezentáciu Daniel 10:2-3, 12-14 (ROH)");
    await page.locator('[data-role="ai-send"]').click();

    // The guard ends the turn with the Slovak give-up message as a normal
    // assistant bubble (NOT the ai-error panel — the backend is healthy).
    const assistantMsg = page.locator(
      '[data-role="ai-message"][data-message-role="assistant"]',
    );
    await expect(assistantMsg).toContainText("Skús zadanie zjednodušiť", {
      timeout: 30_000,
    });

    // A give-up is not an error: the error panel must never appear.
    await expect(page.locator('[data-role="ai-error"]')).not.toBeVisible();

    expect(consoleMessages).toEqual([]);
  });
});
