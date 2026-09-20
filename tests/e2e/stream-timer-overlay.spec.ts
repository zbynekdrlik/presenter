import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #785: the OBS timer overlay is now a stream-graphics OUTPUT (`slug=timer`),
// designable in the same editor as everything else. `/overlays/timer` redirects
// to `/stream/timer`; the editor's output switcher lets an operator design it.

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const countdownDiv = '[data-role="stream-element-countdown"]';
const countdownContent = '[data-role="stream-countdown-content"]';

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

type ElementDef = {
  id: number;
  props: Record<string, any> & { kind: string };
};
type SceneDef = { id: number; name: string; kind: string; elements: ElementDef[] };
type OutputDef = { activeSceneId: number | null; scenes: SceneDef[] };

async function timerDef(page: Page): Promise<OutputDef> {
  const res = await page.request.get(
    new URL("/stream/api/outputs/timer/def", baseURL).toString(),
    { timeout: 30_000 },
  );
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

/** Ratio of computed `letter-spacing` to `font-size` on the countdown element. */
async function letterSpacingRatio(page: Page): Promise<number> {
  const div = page.locator(countdownDiv);
  await div.waitFor({ timeout: 15_000 });
  return div.evaluate((el) => {
    const cs = getComputedStyle(el);
    const ls = parseFloat(cs.letterSpacing); // px (NaN if "normal")
    const fs = parseFloat(cs.fontSize); // px
    return ls / fs;
  });
}

test("legacy /overlays/timer redirects to /stream/timer and renders the seeded countdown", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);

  await page.goto(`${baseURL}/overlays/timer`);
  // The 302 is followed by the browser → we land on the stream output page.
  expect(page.url()).toContain("/stream/timer");

  // The seeded countdown element renders (its content span always exists).
  await page.waitForSelector(countdownContent, { timeout: 15_000 });

  // The seeded overlay look: letter-spacing 0.08em (ratio ≈ 0.08 of font-size).
  const ratio = await letterSpacingRatio(page);
  expect(ratio).toBeGreaterThan(0.06);
  expect(ratio).toBeLessThan(0.1);

  expect(errors, "browser console must be clean").toEqual([]);
});

test("output switcher: design the timer countdown and it reflects on /stream/timer", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);

  // Open the editor on the default `stream` output, then SWITCH to `timer`.
  await page.goto(`${baseURL}/ui/stream`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.locator('[data-role="stream-editor"]').waitFor({ timeout: 30_000 });

  await page
    .locator('[data-role="stream-output-select"]')
    .selectOption("timer");

  // The timer output's base scene "Timer" + its countdown element must load.
  const def = await (async () => {
    let d: OutputDef | undefined;
    await expect
      .poll(async () => {
        d = await timerDef(page);
        return d.scenes.length === 1 && d.scenes[0].elements.length === 1;
      })
      .toBe(true);
    return d!;
  })();
  const sceneId = String(def.scenes[0].id);
  const elementId = String(def.scenes[0].elements[0].id);

  // Open the Timer base scene panel + select the countdown element.
  await page
    .locator(`[data-role="stream-scene"][data-scene-id="${sceneId}"] [data-role="stream-scene-edit"]`)
    .click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
  await page
    .locator(`[data-role="stream-element"][data-element-id="${elementId}"] [data-role="stream-element-select"]`)
    .click();
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });

  // Change size + letter spacing + enable a shadow, then save.
  const tsCd = '[data-role="stream-ts-countdown"] ';
  await page.locator(`${tsCd}[data-role="stream-ts-size"]`).fill("10");
  await page.locator(`${tsCd}[data-role="stream-ts-letter-spacing"]`).fill("0.2");
  await page.locator(`${tsCd}[data-role="stream-ts-shadow-enable"]`).check();
  await page.locator(`${tsCd}[data-role="stream-ts-shadow-blur"]`).fill("8");
  await page.locator('[data-role="stream-prop-save"]').click();

  // The edit persists into the timer output's def.
  await expect
    .poll(async () => {
      const el = (await timerDef(page)).scenes[0].elements[0];
      const s = el.props.style as Record<string, any>;
      return s.letterSpacingEm === 0.2 && s.sizePct === 10 && !!s.shadow;
    })
    .toBe(true);

  // And it renders on the OBS output page.
  await page.goto(`${baseURL}/stream/timer`);
  await page.waitForSelector(countdownContent, { timeout: 15_000 });
  const ratio = await letterSpacingRatio(page);
  expect(ratio).toBeGreaterThan(0.18);
  expect(ratio).toBeLessThan(0.22);

  expect(errors, "browser console must be clean").toEqual([]);
});
