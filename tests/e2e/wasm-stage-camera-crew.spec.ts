/**
 * E2E spec for /ui/camera — camera-crew layout.
 *
 * Three scenarios:
 *  1. Pinned layout: changing the global stage layout via POST /stage/layout
 *     must NOT flip the camera page away from "camera-crew".
 *  2. ON AIR indicator: the indicator reacts to broadcast.set_live commands
 *     sent via the Companion WebSocket.
 *  3. Group label content: after setting a known slide as current, the
 *     camera-crew current pill must render the slide's group name.
 */

import { test, expect } from "@playwright/test";
import WebSocket from "ws";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(
  page: import("@playwright/test").Page,
  extraAllowed: RegExp[] = [],
): string[] {
  const messages: string[] = [];
  const allowed = [...ALLOWED_CONSOLE_NOISE, ...extraAllowed];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!allowed.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

/** Build a Companion WebSocket helper for sending commands. */
function createCompanionSocket(url: string) {
  const socket = new WebSocket(url);

  const waitForMessage = (
    predicate: (msg: Record<string, unknown>) => boolean,
    timeoutMs = 5_000,
  ) =>
    new Promise<Record<string, unknown>>((resolve, reject) => {
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error("Timed out waiting for expected Companion message"));
      }, timeoutMs);

      const cleanup = () => {
        clearTimeout(timeout);
        socket.off("message", handleMessage);
      };

      const handleMessage = (raw: WebSocket.RawData) => {
        try {
          const parsed = JSON.parse(raw.toString());
          if (predicate(parsed)) {
            cleanup();
            resolve(parsed);
          }
        } catch (error) {
          cleanup();
          reject(error as Error);
        }
      };

      socket.on("message", handleMessage);
    });

  async function handshake() {
    await new Promise<void>((resolve, reject) => {
      socket.once("open", () => {
        socket.send(
          JSON.stringify({
            type: "hello",
            client: "Playwright",
            instanceName: "camera-crew-spec",
          }),
        );
        resolve();
      });
      socket.once("error", (err) => reject(err));
    });

    await waitForMessage((msg) => msg.type === "welcome");
    await waitForMessage((msg) => msg.type === "variables");
  }

  async function sendCommand(
    command: string,
    payload: Record<string, unknown> = {},
  ) {
    socket.send(JSON.stringify({ type: "command", command, payload }));
    return waitForMessage(
      (msg) =>
        (msg.type === "ack" && msg.command === command) || msg.type === "error",
    );
  }

  return { socket, handshake, sendCommand };
}

let serverHandle: ServerHandle | undefined;
let baseURL = "";
let companionWsURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);

  // Enable Companion WebSocket so broadcast.set_live commands can be sent.
  const companionPort = cfg.port + 100;
  const resp = await fetch(new URL("/settings/features", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      companionEnabled: true,
      companionPort,
    }),
  });
  if (!resp.ok) {
    throw new Error(
      `Failed to enable Companion WebSocket (${resp.status})`,
    );
  }

  const base = new URL(baseURL);
  const wsOrigin = `${base.protocol.replace("http", "ws")}//${base.hostname}:${companionPort}`;
  companionWsURL = `${wsOrigin}/companion/ws`;
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

// ─── Scenario 1: Pinned layout ───────────────────────────────────────────────

test("pinned layout — operator switch does not flip camera view", async ({
  page,
}) => {
  const consoleMessages = collectConsoleErrors(page);

  await page.goto(new URL("/ui/camera", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });

  // Wait for WASM to boot and set body attributes.
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="camera-crew"]', {
    timeout: 10_000,
  });

  // Confirm the camera page has loaded with the correct pinned layout.
  await expect(page.locator("body")).toHaveAttribute(
    "data-layout-code",
    "camera-crew",
  );

  // The version label is rendered inside the footer's connection span.
  // VersionLabel uses data-testid="version" per project standard.
  await expect(
    page.locator('[data-testid="version"]').first(),
  ).toBeVisible({ timeout: 15_000 });

  // Switch the global stage layout away from camera-crew via the REST API.
  // POST /stage/layout body: { "code": "<layout>" }
  const flip = await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "preach" } },
  );
  expect(flip.ok()).toBeTruthy();

  // Give the WASM event handler time to react (it should ignore this event).
  await page.waitForTimeout(800);

  // The camera page must still be pinned — body attribute must NOT change.
  await expect(page.locator("body")).toHaveAttribute(
    "data-layout-code",
    "camera-crew",
  );

  // Core structural elements must be visible.
  await expect(
    page.locator(".stage__camera-crew__current"),
  ).toBeVisible();
  await expect(
    page.locator(".stage__camera-crew__footer"),
  ).toBeVisible();

  // Console must be clean (checked last, after all UI interactions).
  expect(consoleMessages).toEqual([]);
});

// ─── Scenario 2: ON AIR indicator reacts to BroadcastLive toggle ─────────────

test("ON AIR indicator reacts to BroadcastLive toggle via Companion", async ({
  page,
}) => {
  const consoleMessages = collectConsoleErrors(page);

  await page.goto(new URL("/ui/camera", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });

  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Wait for the WS connection to be established before testing.
  await page.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  const onAir = page.locator('[data-testid="camera-crew-on-air"]');
  await expect(onAir).toBeVisible();

  // Initially broadcast is OFF — the is-on class must not be present.
  await expect(onAir).not.toHaveClass(/is-on/);

  // Connect to Companion and toggle broadcast live ON.
  const { socket, handshake, sendCommand } = createCompanionSocket(
    companionWsURL,
  );
  await handshake();

  const enableResult = await sendCommand("broadcast.set_live", {
    enabled: true,
  });
  expect(enableResult.type).toBe("ack");

  // Wait for the WASM event handler to receive the BroadcastLive event.
  await page.waitForFunction(
    () =>
      document
        .querySelector('[data-testid="camera-crew-on-air"]')
        ?.classList.contains("is-on"),
    { timeout: 5_000 },
  );

  await expect(onAir).toHaveClass(/is-on/);

  // Toggle broadcast live OFF.
  const disableResult = await sendCommand("broadcast.set_live", {
    enabled: false,
  });
  expect(disableResult.type).toBe("ack");

  await page.waitForFunction(
    () =>
      !document
        .querySelector('[data-testid="camera-crew-on-air"]')
        ?.classList.contains("is-on"),
    { timeout: 5_000 },
  );

  await expect(onAir).not.toHaveClass(/is-on/);

  // Console must be clean (checked last, after all UI interactions).
  expect(consoleMessages).toEqual([]);

  socket.close();
});

// ─── Scenario 3: Group label content propagates to camera-crew pill ───────────

test("renders seeded current group label after slide-state set", async ({
  page,
}) => {
  const consoleErrors = collectConsoleErrors(page, [/favicon\.ico/i]);

  // ── Find a presentation that has at least one slide with an explicit group ──
  const libsResp = await page.request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(libsResp.ok()).toBeTruthy();
  const libs = (await libsResp.json()) as Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }>;

  expect(libs.length).toBeGreaterThan(0);

  type SlideData = {
    id: string;
    order: number;
    content: {
      group?: { name: string };
    };
  };
  type PresDetailData = {
    presentation: {
      id: string;
      slides: SlideData[];
    };
  };

  let targetPresentationId: string | null = null;
  let targetSlideId: string | null = null;
  let expectedGroupName: string | null = null;

  // Search libraries in order until we find a grouped slide.
  outer: for (const lib of libs) {
    for (const pres of lib.presentations) {
      const detailResp = await page.request.get(
        new URL(`/presentations/${pres.id}`, baseURL).toString(),
      );
      if (!detailResp.ok()) continue;
      const detail = (await detailResp.json()) as PresDetailData;
      const slides = detail.presentation.slides;

      // Find the first slide that has an explicit group label.
      // resolve_sequence propagates groups forward, so even a slide without an
      // explicit group will show the inherited group in the snapshot. But we
      // need at least one slide with content.group set so there IS a group.
      const groupedSlide = slides.find((s) => s.content.group?.name);
      if (groupedSlide) {
        targetPresentationId = detail.presentation.id;
        targetSlideId = groupedSlide.id;
        expectedGroupName = groupedSlide.content.group!.name;
        break outer;
      }
    }
  }

  expect(targetPresentationId).toBeTruthy();
  expect(targetSlideId).toBeTruthy();
  expect(expectedGroupName).toBeTruthy();

  // ── Set the stage state so the grouped slide is the current slide ──────────
  const stateResp = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: targetPresentationId,
        currentSlideId: targetSlideId,
      },
    },
  );
  expect(stateResp.status()).toBe(204);

  // ── Navigate to /ui/camera and wait for WASM ready ────────────────────────
  await page.goto(new URL("/ui/camera", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });

  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Wait for the stage WebSocket to connect so snapshot updates arrive.
  await page.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // ── Assert the current group pill shows the expected group name ───────────
  // The component renders content.group.name as text; text-transform:uppercase
  // is CSS-only and does NOT affect textContent.
  const currentPill = page.locator(".stage__camera-crew__current");
  await expect(currentPill).toBeVisible();

  // Poll until the snapshot from the WS arrives and the pill is non-empty.
  await expect(currentPill).not.toHaveText("", { timeout: 10_000 });

  const renderedText = (await currentPill.textContent())?.trim() ?? "";
  expect(renderedText).toBe(expectedGroupName);

  // ── Sanity: future-groups strip is rendered (may have 0..3 children) ──────
  const futureStrip = page.locator(".stage__camera-crew__future");
  await expect(futureStrip).toBeVisible();

  // ── Console must be clean ─────────────────────────────────────────────────
  expect(consoleErrors).toEqual([]);
});
