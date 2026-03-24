/**
 * WASM AI Chat Tab Tests
 *
 * Tests the AI chat tab UI: navigation, settings panel, message input,
 * error handling, conversation clearing, and proxy status display.
 *
 * Note: These tests do NOT test actual AI responses (which require
 * an authenticated Claude proxy). They verify the UI behavior,
 * API integration plumbing, and error handling.
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
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

async function initPage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
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

    // Should show error (no authenticated AI backend in test)
    const error = page.locator('[data-role="ai-error"]');
    await expect(error).toBeVisible({ timeout: 15_000 });
    await expect(error).toContainText("Failed to get AI response");
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
    await navigateToAi(page);

    // Send a message (will produce an error in test environment)
    const textarea = page.locator('[data-role="ai-input"]');
    await textarea.fill("test clear");
    await page.locator('[data-role="ai-send"]').click();

    // Wait for user message to appear
    const userMsg = page.locator(
      '[data-role="ai-message"][data-message-role="user"]',
    );
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

test.describe("AI Proxy Status", () => {
  test("proxy section visible in settings when binary not found", async ({
    page,
  }) => {
    await navigateToAi(page);

    const settingsToggle = page.locator('[data-role="ai-settings-toggle"]');
    await settingsToggle.click();

    // In E2E test environment, binary is not alongside the test server
    // So we should see either the proxy controls or the "not found" message
    const proxyTitle = page.getByText("Built-in Proxy (CLIProxyAPI)");
    await expect(proxyTitle).toBeVisible();
  });
});

test.describe("AI Chat Connection Status", () => {
  test("connection status indicator is visible", async ({ page }) => {
    await navigateToAi(page);

    const statusDot = page.locator('[data-role="ai-chat"] .ai-chat__status');
    await expect(statusDot).toBeVisible();
  });

  test("status endpoint returns valid response", async ({ page }) => {
    // Direct API test
    const response = await page.request.get(`${baseURL}/ai/status`);
    expect(response.ok()).toBe(true);

    const data = await response.json();
    expect(data).toHaveProperty("connected");
    expect(data).toHaveProperty("proxy");
    expect(data.proxy).toHaveProperty("running");
    expect(data.proxy).toHaveProperty("binaryFound");
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
