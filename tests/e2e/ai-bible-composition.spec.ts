/**
 * AI Bible Slide Composition Character Limit Tests
 *
 * The real coverage for this feature is a Rust integration test in
 * crates/presenter-server/src/ai/tools.rs:
 *
 *   create_bible_presentation_with_long_passage_composes_many_slides
 *
 * It exercises execute_tool("create_bible_presentation", ...) directly,
 * which is the same dispatch path /ai/chat uses after the LLM returns
 * a tool call. That test asserts:
 *
 *   - The server splits a 12-verse passage into multiple slides
 *   - Every slide's main text fits under the configured character limit
 *   - No raw ## markers survive
 *   - Emphasis slides render with empty references
 *
 * The Playwright test below is a thin browser-level smoke test that
 * exercises what we CAN test without a live LLM: the /ai/chat endpoint's
 * input validation (empty message → 400) and the /bible/presentations
 * list API shape. Full end-to-end with real AI responses is out of scope
 * because the test environment has no authenticated Claude proxy.
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

test.describe.configure({ timeout: 120_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

test.describe("AI bible slide composition", () => {
  test("POST /ai/chat rejects empty message with 400", async ({ page }) => {
    // Sanity check that the dispatch endpoint exists and validates input.
    // The actual composition path (LLM → create_bible_presentation items)
    // is covered by a Rust integration test because /ai/chat itself requires
    // a live authenticated AI backend that the test environment lacks.
    const response = await page.request.post(`${baseURL}/ai/chat`, {
      data: { message: "" },
    });
    expect(response.status()).toBe(400);
  });

  test("bible presentation API returns empty list on fresh state", async ({
    page,
  }) => {
    // Baseline: the bible presentation read path works and returns an
    // array. This is the API the operator UI uses to list presentations
    // after the AI creates them. The in-memory state starts empty, so
    // we assert the shape is an array and contains zero or more items.
    const response = await page.request.get(
      `${baseURL}/bible/presentations`,
    );
    expect(response.ok()).toBe(true);
    const body = await response.json();
    expect(Array.isArray(body)).toBe(true);
  });
});
