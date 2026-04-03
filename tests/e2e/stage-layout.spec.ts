import { test, expect, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;

test.describe.configure({ timeout: 180_000 });

/** Edge-case group names for layout testing */
const TEST_SLIDES = [
  { main: "Hosana, Hosana\nHosana v výšinách", group: "Chorus" },
  { main: "Požehnaný kto prichádza\nv mene Pánovom", group: "Žalm Ť" },
  {
    main: "Sláva Ti Lev z Júdy\nnech teraz reve Lev",
    group: "Muži // Ženy",
  },
  { main: "Veľký je náš Boh\na hoden chvály", group: "Všetci" },
  { main: "Short line", group: "A" },
];

let testPresentationId: string;
let testSlideIds: string[];

async function openStageDisplay(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  return stagePage;
}

async function triggerSlide(
  context: BrowserContext,
  currentIdx: number,
  nextIdx?: number,
) {
  const data: Record<string, string | null> = {
    presentationId: testPresentationId,
    currentSlideId: testSlideIds[currentIdx],
    nextSlideId: nextIdx != null ? testSlideIds[nextIdx] : null,
  };
  await context.request.post(new URL("/stage/state", baseURL).toString(), {
    data,
  });
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(port, dbUrl, config.oscPort);

  // Create test library
  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "_E2E Layout Test" }),
  });
  const lib = await libResp.json();

  // Create test presentation with edge-case slides
  const presResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Layout Edge Cases",
        slides: TEST_SLIDES,
      }),
    },
  );
  const presData = await presResp.json();
  testPresentationId = presData.presentation.id;
  testSlideIds = presData.presentation.slides.map(
    (s: { id: string }) => s.id,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

// ─── Diacritics visibility ───────────────────────────────────────────────

test("group pill renders diacritics without clipping (Ž, Ť)", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Trigger slide with "Žalm Ť" group (háčeks) and "Muži // Ženy" as next
  await triggerSlide(context, 1, 2);
  await stagePage.waitForTimeout(2_000);

  // Current group should show "ŽALM Ť" (uppercase via CSS)
  const currentPill = stagePage.locator(
    ".stage__current-group .stage__group-pill",
  );
  await expect(currentPill).toBeVisible();
  const currentText = await currentPill.textContent();
  // text-transform:uppercase means source "Žalm Ť" renders as "ŽALM Ť"
  // Verify the diacritics characters are present (not stripped/clipped in DOM)
  expect(currentText).toContain("Ž");
  expect(currentText).toContain("Ť");

  // Next group should show "MUŽI // ŽENY"
  const nextPill = stagePage.locator(".stage__next-group .stage__group-pill");
  await expect(nextPill).toBeVisible();
  const nextText = await nextPill.textContent();
  expect(nextText).toContain("Ž");

  // Verify diacritics are not visually clipped by measuring:
  // The rendered text height (scrollHeight) should equal the element's clientHeight.
  // If diacritics were clipped, scrollHeight > clientHeight.
  const currentOverflow = await currentPill.evaluate((el) => {
    return el.scrollHeight > el.clientHeight;
  });
  expect(currentOverflow).toBe(false);

  const nextOverflow = await nextPill.evaluate((el) => {
    return el.scrollHeight > el.clientHeight;
  });
  expect(nextOverflow).toBe(false);

  await stagePage.close();
});

// ─── Autofit fills box ───────────────────────────────────────────────────

test("autofit scales group pill text to fill box height", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Trigger slide with "Chorus" group (no diacritics — maximum fill)
  await triggerSlide(context, 0, 1);
  await stagePage.waitForTimeout(2_000);

  // Measure: font-size relative to container height
  // With line-height 0.95, autofit should pick font-size ≈ containerH / 0.95
  // So font-size should be > 90% of container height
  const metrics = await stagePage.evaluate(() => {
    const container = document.querySelector(".stage__current-group");
    const pill = container?.querySelector(".stage__group-pill");
    if (!container || !pill) return null;
    const containerH = container.getBoundingClientRect().height;
    const fontSize = parseFloat(getComputedStyle(pill).fontSize);
    return { containerH, fontSize, ratio: fontSize / containerH };
  });

  expect(metrics).not.toBeNull();
  // Font-size should be > 90% of container height (autofit maximizes)
  expect(metrics!.ratio).toBeGreaterThan(0.9);

  await stagePage.close();
});

test("autofit scales single-character group to fill box", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Trigger slide with "A" group (single char — should be huge)
  await triggerSlide(context, 4, 0);
  await stagePage.waitForTimeout(2_000);

  const metrics = await stagePage.evaluate(() => {
    const container = document.querySelector(".stage__current-group");
    const pill = container?.querySelector(".stage__group-pill");
    if (!container || !pill) return null;
    const containerH = container.getBoundingClientRect().height;
    const fontSize = parseFloat(getComputedStyle(pill).fontSize);
    return { containerH, fontSize, ratio: fontSize / containerH };
  });

  expect(metrics).not.toBeNull();
  // Single char should fill the box height
  expect(metrics!.ratio).toBeGreaterThan(0.9);

  await stagePage.close();
});

// ─── Slide text with diacritics ──────────────────────────────────────────

test("slide text renders diacritics correctly", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  // Trigger slide with diacritics in main text
  await triggerSlide(context, 1, 3);
  await stagePage.waitForTimeout(2_000);

  const slideText = stagePage.locator(
    ".stage__current-slide .stage__slide-text",
  );
  await expect(slideText).toBeVisible();
  const text = await slideText.textContent();

  // Verify diacritics in slide text
  expect(text).toContain("Požehnaný");
  expect(text).toContain("prichádza");
  expect(text).toContain("Pánovom");

  await stagePage.close();
});

// ─── Slide text fills box ────────────────────────────────────────────────

test("slide text autofit fills box height with no overflow", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Trigger 2-line slide with diacritics (Požehnaný kto prichádza / v mene Pánovom)
  await triggerSlide(context, 1, 2);
  await stagePage.waitForTimeout(2_000);

  const metrics = await stagePage.evaluate(() => {
    const container = document.querySelector(".stage__current-slide");
    const text = container?.querySelector(".stage__slide-text");
    if (!container || !text) return null;
    const containerH = container.getBoundingClientRect().height;
    const fontSize = parseFloat(getComputedStyle(text).fontSize);
    const lineHeight = parseFloat(getComputedStyle(text).lineHeight);
    const overflows =
      text.scrollHeight > (text as HTMLElement).clientHeight ||
      text.scrollWidth > (text as HTMLElement).clientWidth;
    return { containerH, fontSize, lineHeight, overflows };
  });

  expect(metrics).not.toBeNull();
  // Text should not overflow
  expect(metrics!.overflows).toBe(false);
  // line-height should be close to font-size (line-height: 1.15)
  expect(metrics!.lineHeight / metrics!.fontSize).toBeLessThan(1.2);
  expect(metrics!.lineHeight / metrics!.fontSize).toBeGreaterThan(1.0);
  // Font-size should be substantial relative to container (not tiny)
  expect(metrics!.fontSize / metrics!.containerH).toBeGreaterThan(0.15);

  await stagePage.close();
});

test("single-line slide text maximizes without overflow", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Trigger single-line slide ("Short line")
  await triggerSlide(context, 4, 0);
  await stagePage.waitForTimeout(2_000);

  const metrics = await stagePage.evaluate(() => {
    const container = document.querySelector(".stage__current-slide");
    const text = container?.querySelector(".stage__slide-text");
    if (!container || !text) return null;
    const containerH = container.getBoundingClientRect().height;
    const containerW = container.getBoundingClientRect().width;
    const fontSize = parseFloat(getComputedStyle(text).fontSize);
    const overflows =
      text.scrollHeight > (text as HTMLElement).clientHeight ||
      text.scrollWidth > (text as HTMLElement).clientWidth;
    return { containerH, containerW, fontSize, overflows };
  });

  expect(metrics).not.toBeNull();
  expect(metrics!.overflows).toBe(false);
  // Font should be significantly larger than a default size (autofit working)
  // Width-constrained single lines won't fill height, but should still be big
  expect(metrics!.fontSize).toBeGreaterThan(20);

  await stagePage.close();
});

// ─── Box independence (no layout shifts) ─────────────────────────────────

test("boxes maintain fixed positions regardless of content", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // First: trigger short content
  await triggerSlide(context, 4, 0);
  await stagePage.waitForTimeout(2_000);

  const getBoxPositions = async () => {
    return stagePage.evaluate(() => {
      const boxes = [
        ".stage__current-group",
        ".stage__current-slide",
        ".stage__next-group",
        ".stage__next-slide",
        ".stage__clock",
      ];
      return boxes.map((sel) => {
        const el = document.querySelector(sel);
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { sel, top: Math.round(r.top), height: Math.round(r.height) };
      });
    });
  };

  const positionsShort = await getBoxPositions();

  // Second: trigger long content with diacritics
  await triggerSlide(context, 1, 2);
  await stagePage.waitForTimeout(2_000);

  const positionsLong = await getBoxPositions();

  // All boxes should be at the SAME positions regardless of content
  for (let i = 0; i < positionsShort.length; i++) {
    expect(positionsShort[i]).not.toBeNull();
    expect(positionsLong[i]).not.toBeNull();
    if (positionsShort[i] && positionsLong[i]) {
      expect(positionsLong[i]!.top).toBe(positionsShort[i]!.top);
      expect(positionsLong[i]!.height).toBe(positionsShort[i]!.height);
    }
  }

  await stagePage.close();
});

// ─── Status bar autofit ──────────────────────────────────────────────────

test("status bar elements autofit to bar height", async ({ context }) => {
  const stagePage = await openStageDisplay(context);
  await stagePage.waitForTimeout(2_000);

  const metrics = await stagePage.evaluate(() => {
    const clock = document.querySelector(".stage__clock");
    const live = document.querySelector(".stage__live-pill");
    const conn = document.querySelector(".stage__connection");
    if (!clock || !live || !conn) return null;

    return {
      clockH: clock.getBoundingClientRect().height,
      clockFontSize: parseFloat(getComputedStyle(clock).fontSize),
      liveH: live.getBoundingClientRect().height,
      liveFontSize: parseFloat(getComputedStyle(live).fontSize),
      connH: conn.getBoundingClientRect().height,
      connFontSize: parseFloat(getComputedStyle(conn).fontSize),
    };
  });

  expect(metrics).not.toBeNull();
  // Each status box has its own height. Font-size should be > 30% of its box.
  expect(metrics!.clockFontSize / metrics!.clockH).toBeGreaterThan(0.3);
  expect(metrics!.liveFontSize / metrics!.liveH).toBeGreaterThan(0.3);
  expect(metrics!.connFontSize / metrics!.connH).toBeGreaterThan(0.3);

  await stagePage.close();
});

// ─── Console clean ───────────────────────────────────────────────────────

test("stage display has no console errors", async ({ context }) => {
  const stagePage = await openStageDisplay(context);
  const consoleMessages: string[] = [];
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Trigger a few slides to exercise the layout
  await triggerSlide(context, 0, 1);
  await stagePage.waitForTimeout(1_000);
  await triggerSlide(context, 1, 2);
  await stagePage.waitForTimeout(1_000);
  await triggerSlide(context, 2, 3);
  await stagePage.waitForTimeout(1_000);

  // Filter allowed warnings
  const ALLOWED = [/ResizeObserver loop/];
  const real = consoleMessages.filter(
    (m) => !ALLOWED.some((r) => r.test(m)),
  );
  expect(real).toEqual([]);

  await stagePage.close();
});
