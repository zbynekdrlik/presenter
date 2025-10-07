import { test, expect } from '@playwright/test';
import type { APIRequestContext, Page, TestInfo } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

const EXPLICIT_STAGE_URL = process.env.STAGE_URL;
let stageBaseUrl: string | undefined = EXPLICIT_STAGE_URL?.replace(/\/$/, '');
let serverHandle: ServerHandle | undefined;
let dbUrl: string;
let oscPort: number | undefined;
let port: number | undefined;
const LINE_LIMIT = 2.2;
const FONT_FALLBACK_RATIO = 1.12;

type SlideMatchCriteria = {
  includes: string;
  field?: 'main' | 'stage' | 'translation';
};

type NextMatchCriteria = {
  includes?: string;
  field?: 'main' | 'stage' | 'translation';
  offset?: number;
};

type StageSlideConfig = {
  label: string;
  presentationName: string;
  current: SlideMatchCriteria;
  next?: NextMatchCriteria;
  minFontPx?: number;
};

type LayoutScenario = {
  layout: string;
  elements: string[];
  slides: StageSlideConfig[];
};

const VIEWPORTS = [
  { name: 'Retina', width: 2880, height: 1800 },
  { name: '1080p', width: 1920, height: 1080 },
  { name: '900p', width: 1600, height: 900 },
  { name: '720p', width: 1280, height: 720 },
  { name: '4x3', width: 1024, height: 768 },
] as const;

const LAYOUT_SCENARIOS: LayoutScenario[] = [
  {
    layout: 'worship-snv',
    elements: ['current-text', 'next-text'],
    slides: [
      {
        label: 'Nehladam width check (Nezáleží…)',
        presentationName: 'Nehľadám svoje',
        current: { includes: 'Nezáleží už na tom čo chcem ja' },
        next: { includes: 'Keď vôľu Tvoju hľadám' },
        minFontPx: 22,
      },
      {
        label: 'Nehladam chorus',
        presentationName: 'Nehľadám svoje',
        current: { includes: 'Vyvýšim Ťa nad túžby' },
        next: { includes: 'Chcem žiť pre Teba' },
        minFontPx: 22,
      },
      {
        label: 'Boh je so mnou long line',
        presentationName: 'Boh je so mnou',
        current: { includes: 'čo zasľúbil slovom, naozaj vykoná' },
        next: { includes: 'Nemusím sa báť, pri mne blízko je' },
        minFontPx: 22,
      },
      {
        label: 'Boh je so mnou stage text',
        presentationName: 'Boh je so mnou',
        current: { includes: 'On je vždy so mnou, óó', field: 'stage' },
        next: { offset: 1 },
        minFontPx: 22,
      },
    ],
  },
  {
    layout: 'worship-pp',
    elements: ['current-main', 'next-main'],
    slides: [
      {
        label: 'Nehladam chorus',
        presentationName: 'Nehľadám svoje',
        current: { includes: 'Vyvýšim Ťa nad túžby' },
        next: { includes: 'Chcem žiť pre Teba' },
        minFontPx: 22,
      },
      {
        label: 'Boh je so mnou stage note',
        presentationName: 'Boh je so mnou',
        current: { includes: 'On je vždy so mnou, óó', field: 'stage' },
        next: { offset: 1 },
        minFontPx: 22,
      },
      {
        label: 'Boh je so mnou long line',
        presentationName: 'Boh je so mnou',
        current: { includes: 'čo zasľúbil slovom, naozaj vykoná' },
        next: { includes: 'Nemusím sa báť, pri mne blízko je' },
        minFontPx: 22,
      },
    ],
  },
] as const;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({}, testInfo: TestInfo) => {
  if (stageBaseUrl) {
    return;
  }
  const config = deriveTestConfig(testInfo);
  port = config.port;
  dbUrl = config.dbUrl;
  oscPort = config.oscPort;
  stageBaseUrl = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

type ResolvedScenario = {
  presentationId: string;
  currentSlideId: string;
  nextSlideId?: string;
};

const scenarioCache = new Map<string, ResolvedScenario>();

function getSlideFieldValue(slide: any, field: 'main' | 'stage' | 'translation'): string {
  const content = slide?.content ?? {};
  const target = content[field] ?? {};
  return typeof target.value === 'string' ? target.value : '';
}

function buildScenarioCacheKey(config: StageSlideConfig): string {
  return JSON.stringify({
    presentationName: config.presentationName,
    current: config.current,
    next: config.next ?? null,
  });
}

function findSlideIndex(slides: any[], criteria: SlideMatchCriteria): number {
  const field = criteria.field ?? 'main';
  const needle = criteria.includes;
  const index = slides.findIndex((slide) => getSlideFieldValue(slide, field).includes(needle));
  if (index === -1) {
    throw new Error(`Slide containing "${needle}" in ${field} not found`);
  }
  return index;
}

async function resolveScenarioSlides(
  request: APIRequestContext,
  config: StageSlideConfig,
): Promise<ResolvedScenario> {
  const cacheKey = buildScenarioCacheKey(config);
  const cached = scenarioCache.get(cacheKey);
  if (cached) {
    return cached;
  }

  if (!stageBaseUrl) {
    throw new Error('Stage base URL not initialised');
  }

  const searchUrl = new URL('/search', `${stageBaseUrl}/`);
  searchUrl.searchParams.set('query', config.presentationName);
  const searchResponse = await request.get(searchUrl.toString());
  expect(searchResponse.ok(), `search failed for ${config.presentationName}`).toBeTruthy();
  const searchResults = (await searchResponse.json()) as any[];
  const presentation = searchResults.find(
    (result) =>
      result &&
      result.kind === 'presentation' &&
      result.presentationName === config.presentationName,
  );

  if (!presentation) {
    throw new Error(`Presentation "${config.presentationName}" not found in search results`);
  }

  const detailUrl = new URL(`/presentations/${presentation.presentationId}`, `${stageBaseUrl}/`);
  const detailResponse = await request.get(detailUrl.toString());
  expect(detailResponse.ok(), `load presentation ${presentation.presentationId} failed`).toBeTruthy();
  const detailJson = await detailResponse.json();
  const slides = detailJson?.presentation?.slides;
  if (!Array.isArray(slides) || slides.length === 0) {
    throw new Error(`Presentation "${config.presentationName}" has no slides`);
  }

  const currentIndex = findSlideIndex(slides, config.current);
  const currentSlide = slides[currentIndex];

  let nextSlideId: string | undefined;
  if (config.next?.includes) {
    const nextIndex = findSlideIndex(slides, {
      includes: config.next.includes,
      field: config.next.field,
    });
    nextSlideId = slides[nextIndex]?.id;
  } else {
    const offset = config.next?.offset ?? 1;
    const candidateIndex = currentIndex + offset;
    if (candidateIndex >= 0 && candidateIndex < slides.length) {
      nextSlideId = slides[candidateIndex]?.id;
    }
  }

  const resolved: ResolvedScenario = {
    presentationId: presentation.presentationId,
    currentSlideId: currentSlide.id,
    nextSlideId,
  };
  scenarioCache.set(cacheKey, resolved);
  return resolved;
}

async function postJson(
  request: APIRequestContext,
  path: string,
  body: Record<string, unknown>,
): Promise<void> {
  if (!stageBaseUrl) {
    throw new Error('Stage base URL not initialised');
  }
  const target = new URL(path, `${stageBaseUrl}/`).toString();
  const response = await request.post(target, { data: body });
  expect(response.ok(), `POST ${path} failed (${response.status()})`).toBeTruthy();
}

async function collectMetrics(page: Page, elementIds: string[]) {
  return page.evaluate(
    ({ ids, fallbackRatio }) => {
      const res = ids.map((id) => {
        const element = document.getElementById(id);
        if (!element) {
          return { id, present: false };
        }
        const style = window.getComputedStyle(element);
        const fontSizePx = parseFloat(style.fontSize) || 0;
        let lineHeightPx = parseFloat(style.lineHeight);
        if (!Number.isFinite(lineHeightPx) || lineHeightPx <= 0) {
          lineHeightPx = fontSizePx * fallbackRatio;
        }
        const container = element.parentElement || element;
        const containerRect = container.getBoundingClientRect();
        const elementRect = element.getBoundingClientRect();
        const containerStyle = window.getComputedStyle(container);
        const padL = parseFloat(containerStyle.paddingLeft) || 0;
        const padR = parseFloat(containerStyle.paddingRight) || 0;
        const contentLeft = containerRect.left + padL;
        const contentRight = containerRect.left + containerRect.width - padR;
        const contentWidth = Math.max(0, contentRight - contentLeft);
        const usedWidth = elementRect.width;
        const widthCoverage = contentWidth > 0 ? usedWidth / contentWidth : 0;
        const leftGutter = Math.max(0, elementRect.left - contentLeft);
        const rightGutter = Math.max(0, contentRight - (elementRect.left + elementRect.width));
        const paddingTop = parseFloat(style.paddingTop) || 0;
        const paddingBottom = parseFloat(style.paddingBottom) || 0;
        const contentHeight = Math.max(0, element.scrollHeight - paddingTop - paddingBottom);
        const lines = lineHeightPx > 0 ? contentHeight / lineHeightPx : 0;
        const rects = Array.from(element.getClientRects()).filter(
          (rect) => rect.width > 1 && rect.height > 1,
        );
        const uniqueLines = Array.from(new Set(rects.map((rect) => rect.top.toFixed(2)))).length;
        return {
          id,
          present: true,
          text: (element.textContent || '').trim(),
          fontSizePx,
          lineHeightPx,
          contentHeight,
          lines,
          rectCount: rects.length,
          uniqueLines,
          clientWidth: element.clientWidth,
          containerWidthPx: containerRect.width,
          elementWidthPx: elementRect.width,
          widthCoverage,
          leftGutterPx: leftGutter,
          rightGutterPx: rightGutter,
          clientHeight: element.clientHeight,
          scrollHeight: element.scrollHeight,
          bodyPaddingLeftPx: parseFloat(getComputedStyle(document.body).paddingLeft) || 0,
          bodyPaddingRightPx: parseFloat(getComputedStyle(document.body).paddingRight) || 0,
          parentHeightPx: element.parentElement?.getBoundingClientRect().height || 0,
          rootHeightPx: (element.closest('.stage__lyrics') as HTMLElement | null)?.getBoundingClientRect().height || 0,
          bottomOverflowPx: Math.max(0, (element.getBoundingClientRect().bottom) - ((element.parentElement?.getBoundingClientRect().bottom) || 0)),
        };
      });
      return res;
    },
    { ids: elementIds, fallbackRatio: FONT_FALLBACK_RATIO },
  );
}

for (const viewport of VIEWPORTS) {
  test.describe(`Stage scaler @ ${viewport.name}`, () => {
    test.use({ viewport: { width: viewport.width, height: viewport.height } });

    for (const scenario of LAYOUT_SCENARIOS) {
      for (const slide of scenario.slides) {
        test(`${scenario.layout} :: ${slide.label}`, async ({ page, request }) => {
          const resolved = await resolveScenarioSlides(request, slide);
          await postJson(request, '/stage/layout', { code: scenario.layout });
          const statePayload: Record<string, unknown> = {
            presentationId: resolved.presentationId,
            currentSlideId: resolved.currentSlideId,
          };
          if (resolved.nextSlideId) {
            statePayload.nextSlideId = resolved.nextSlideId;
          }
          await postJson(request, '/stage/state', statePayload);

        if (!stageBaseUrl) {
          throw new Error('Stage base URL not initialised');
        }
        await page.goto(new URL('/stage', `${stageBaseUrl}/`).toString(), {
          waitUntil: 'domcontentloaded',
        });
        await page.waitForFunction(
          (expected) => window.__presenterStageLayout === expected,
          scenario.layout,
        );
        await page.waitForTimeout(150);

        // For SNV @ Retina, assert the stage consumes the full viewport width (no left-aligned shrink with black gap).
        if (scenario.layout === 'worship-snv' && viewport.width === 2880 && viewport.height === 1800) {
          const widths = await page.evaluate(() => {
            const stageBody = document.querySelector('main.stage__body');
            const lyrics = document.querySelector('.stage__lyrics');
            const bodyRect = document.body.getBoundingClientRect();
            const stageRect = stageBody ? (stageBody as HTMLElement).getBoundingClientRect() : { width: 0 } as DOMRect;
            const lyricsRect = lyrics ? (lyrics as HTMLElement).getBoundingClientRect() : { width: 0 } as DOMRect;
            const viewportWidth = document.documentElement.clientWidth;
            return {
              viewportWidth,
              bodyWidth: bodyRect.width,
              stageBodyWidth: (stageRect as any).width || 0,
              lyricsWidth: (lyricsRect as any).width || 0,
            };
          });
          await test.info().attach('widths', { contentType: 'application/json', body: JSON.stringify(widths) });
          const minWidth = Math.floor(widths.viewportWidth * 0.98);
          expect(widths.bodyWidth).toBeGreaterThanOrEqual(minWidth);
          expect(widths.stageBodyWidth).toBeGreaterThanOrEqual(minWidth);
          expect(widths.lyricsWidth).toBeGreaterThanOrEqual(Math.floor(widths.viewportWidth * 0.96));
          await test.info().attach('stage-screenshot', { body: await page.screenshot({ fullPage: false }), contentType: 'image/png' });
        }

          const metrics = await collectMetrics(page, scenario.elements);
          await test.info().attach('metrics', {
            contentType: 'application/json',
            body: JSON.stringify({ viewport, scenario: scenario.layout, slide: slide.label, metrics }),
          });

          for (const metric of metrics) {
            if (!metric.present || !metric.text) {
              continue;
            }
            expect(
              metric.uniqueLines,
              `unique line count for ${scenario.layout} / ${slide.label} / ${metric.id}`,
            ).toBeLessThanOrEqual(2);
            expect(
              metric.lines,
              `line count for ${scenario.layout} / ${slide.label} / ${metric.id}`,
            ).toBeLessThanOrEqual(LINE_LIMIT);
            expect(
              metric.fontSizePx,
              `font size for ${scenario.layout} / ${slide.label} / ${metric.id}`,
            ).toBeGreaterThanOrEqual(slide.minFontPx ?? 26);

            // Ensure we use horizontal space efficiently for the current line.
            const isCurrent = /current/.test(metric.id);
            const isRetina = viewport.width === 2880 && viewport.height === 1800;
            if (isCurrent && scenario.layout === 'worship-snv' && isRetina) {
              expect(
                metric.widthCoverage,
                `width coverage for ${scenario.layout} / ${slide.label} / ${metric.id}`,
              ).toBeGreaterThanOrEqual(0.98);
              const maxGutterPx = Math.max(10, Math.round((metric.containerWidthPx || 0) * 0.01));
              expect(metric.leftGutterPx).toBeLessThanOrEqual(maxGutterPx);
              expect(metric.rightGutterPx).toBeLessThanOrEqual(maxGutterPx);
              expect(metric.bodyPaddingLeftPx).toBeLessThanOrEqual(12);
              expect(metric.bodyPaddingRightPx).toBeLessThanOrEqual(12);

              // Equal split check (SNV: current/next halves remain stable)
              const next = metrics.find(m => m.id === 'next-text');
              if (next && next.present) {
                const total = (metric.parentHeightPx || 0) + (next.parentHeightPx || 0);
                if (total > 0) {
                  const share = (metric.parentHeightPx || 0) / total;
                  expect(share).toBeGreaterThanOrEqual(0.48);
                  expect(share).toBeLessThanOrEqual(0.52);
                }
              }

              // No overflow of current text outside its half
              expect(metric.bottomOverflowPx).toBeLessThanOrEqual(1);
            }
          }

          await page.waitForTimeout(200);
          const secondPass = await collectMetrics(page, scenario.elements);
          await test.info().attach('metrics-after', {
            contentType: 'application/json',
            body: JSON.stringify({
              viewport,
              scenario: scenario.layout,
              slide: slide.label,
              metrics: secondPass,
            }),
          });
          for (const metric of secondPass) {
            if (!metric.present || !metric.text) {
              continue;
            }
            const first = metrics.find((m) => m.id === metric.id);
            if (!first || !first.present || !first.text) {
              continue;
            }
            expect(
              Math.abs(metric.fontSizePx - first.fontSizePx),
              `font stability for ${scenario.layout} / ${slide.label} / ${metric.id}`,
            ).toBeLessThanOrEqual(0.75);
          }
        });
      }
    }
  });
}
