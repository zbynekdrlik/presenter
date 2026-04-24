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
  // index 5: single-line, 30 chars with diacritics — should auto-break at last space
  { main: "Nad všetkých vyvyšený bude Pán", group: "Auto" },
  // index 6: single-line, 12 chars — below threshold, must NOT break
  { main: "Ježiš je Pán", group: "Auto" },
  // index 7: 28 visible chars with diacritics — bytes > 32 but chars <= 32,
  // must NOT be flagged by the operator overflow warning (char-count fix).
  { main: "Nad všetkých vyvyšený bude P", group: "Auto" },
];

let testPresentationId: string;
let testSlideIds: string[];
let testLibraryId: string;

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
  testLibraryId = lib.id;

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

// ─── Auto-break (single-line slides over threshold) ──────────────────────

test("stage auto-breaks single-line slide over 26 chars", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await triggerSlide(context, 5);
  await stagePage.waitForTimeout(2_000);

  const currentText = await stagePage
    .locator(".stage__current-slide .stage__slide-text")
    .first()
    .evaluate((el) => el.textContent ?? "");

  expect(
    currentText.includes("\n"),
    `Expected a newline to be injected into the slide text, got: ${JSON.stringify(currentText)}`,
  ).toBe(true);

  // Tail-break: the last word "Pán" should be alone on line 2.
  const lines = currentText.split("\n");
  expect(lines.length).toBe(2);
  expect(lines[1].trim()).toBe("Pán");
  // Line 1 must be within the 26-char threshold.
  expect([...lines[0]].length).toBeLessThanOrEqual(26);

  expect(consoleMessages).toEqual([]);
});

test("stage does not break slide below threshold", async ({ context }) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await triggerSlide(context, 6);
  await stagePage.waitForTimeout(2_000);

  const currentText = await stagePage
    .locator(".stage__current-slide .stage__slide-text")
    .first()
    .evaluate((el) => el.textContent ?? "");

  expect(currentText).toBe("Ježiš je Pán");
  expect(currentText.includes("\n")).toBe(false);

  expect(consoleMessages).toEqual([]);
});

// ─── Edge-to-edge layout (issue: maximize lyrics area) ───────────────────

test("stage worship-snv boxes snap to viewport edges", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  // Trigger a slide so boxes render with content (some layouts short-circuit
  // when empty). The selectors we assert on exist regardless of content.
  await triggerSlide(context, 0, 1);
  await stagePage.waitForTimeout(2_000);

  const geom = await stagePage.evaluate(() => {
    const vw = window.innerWidth;
    const read = (sel: string) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return {
        left: Math.round(r.left),
        right: Math.round(vw - r.right),
        width: Math.round(r.width),
      };
    };
    return {
      vw,
      currentSlide: read(".stage__current-slide"),
      nextSlide: read(".stage__next-slide"),
      currentGroup: read(".stage__current-group"),
      currentSong: read(".stage__current-song"),
      nextGroup: read(".stage__next-group"),
      nextSong: read(".stage__next-song"),
    };
  });

  const TOL = 2; // ±2px tolerance for sub-pixel rounding

  // Full-width slides: left edge at 0, right edge at viewport width
  expect(geom.currentSlide).not.toBeNull();
  expect(geom.currentSlide!.left).toBeLessThanOrEqual(TOL);
  expect(geom.currentSlide!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.currentSlide!.width - geom.vw)).toBeLessThanOrEqual(TOL);

  expect(geom.nextSlide).not.toBeNull();
  expect(geom.nextSlide!.left).toBeLessThanOrEqual(TOL);
  expect(geom.nextSlide!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.nextSlide!.width - geom.vw)).toBeLessThanOrEqual(TOL);

  // Left pills: flush left, 50% width
  const halfVw = geom.vw / 2;
  expect(geom.currentGroup).not.toBeNull();
  expect(geom.currentGroup!.left).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.currentGroup!.width - halfVw)).toBeLessThanOrEqual(TOL);

  expect(geom.nextGroup).not.toBeNull();
  expect(geom.nextGroup!.left).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.nextGroup!.width - halfVw)).toBeLessThanOrEqual(TOL);

  // Right pills: flush right, 50% width
  expect(geom.currentSong).not.toBeNull();
  expect(geom.currentSong!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.currentSong!.width - halfVw)).toBeLessThanOrEqual(TOL);

  expect(geom.nextSong).not.toBeNull();
  expect(geom.nextSong!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.nextSong!.width - halfVw)).toBeLessThanOrEqual(TOL);

  await stagePage.close();
});

// ─── Operator overflow warning (byte→char fix) ───────────────────────────

test("operator UI does not flag 28-char diacritic line as overflow", async ({
  context,
}) => {
  const consoleMessages: string[] = [];
  const page = await context.newPage();
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await page.goto(new URL("/ui/operator", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Open the library via the "more" modal (works regardless of favorites).
  await page.locator('[data-role="library-more"]').click();
  await page.waitForFunction(
    () => {
      const modal = document.querySelector('[data-role="library-modal"]');
      return modal && modal.getAttribute("data-open") === "true";
    },
    { timeout: 10_000 },
  );
  // The modal lists all libraries. Find our test library by data-library-id and click it.
  await page
    .locator(
      `[data-role="library-modal-list"] [data-library-id="${testLibraryId}"] .operator__list-button`,
    )
    .click();

  // After selecting from the modal the presentations list should update.
  await page.waitForSelector(
    `[data-role="presentation-item"][data-presentation-id="${testPresentationId}"]`,
    { timeout: 15_000 },
  );
  await page
    .locator(
      `[data-role="presentation-item"][data-presentation-id="${testPresentationId}"]`,
    )
    .click();

  // Wait for slide cards to render.
  await page.waitForSelector(`[data-slide-id="${testSlideIds[7]}"]`, {
    timeout: 15_000,
  });

  const slideCard = page.locator(`[data-slide-id="${testSlideIds[7]}"]`);
  await expect(slideCard).toBeVisible();

  const overflowCount = await slideCard
    .locator(".operator__slide-overflow")
    .count();
  expect(
    overflowCount,
    "28-char diacritic line must not trigger overflow warning after byte->char fix",
  ).toBe(0);

  // Filter known browser-level warnings that are not app errors.
  const ALLOWED = [/integrity.*ignored.*preload/i, /ResizeObserver loop/i];
  const realMessages = consoleMessages.filter(
    (m) => !ALLOWED.some((r) => r.test(m)),
  );
  expect(realMessages).toEqual([]);
});
