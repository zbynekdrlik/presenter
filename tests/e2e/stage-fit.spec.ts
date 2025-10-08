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

async function waitForServerLayout(
  request: APIRequestContext,
  expected: string,
  timeoutMs = 10000,
  intervalMs = 200,
): Promise<void> {
  const start = Date.now();
  for (;;) {
    const res = await request.get(new URL('/stage/layout', `${stageBaseUrl}/`).toString());
    if (res.ok()) {
      const js = await res.json();
      const code = (js?.code || js?.layoutCode || js?.layout_code || '').toString();
      if (code === expected) return;
    }
    if (Date.now() - start > timeoutMs) {
      throw new Error(`Server layout did not switch to '${expected}' in ${timeoutMs}ms`);
    }
    await new Promise(r => setTimeout(r, intervalMs));
  }
}

async function collectMetrics(page: Page, elementIds: string[]) {
  return page.evaluate(
    ({ ids, fallbackRatio }) => {
      function isPageScrollable() {
        const se = document.scrollingElement || document.documentElement;
        const scrollableY = (se?.scrollHeight || 0) > (se?.clientHeight || 0) + 1;
        const scrollableX = (se?.scrollWidth || 0) > (se?.clientWidth || 0) + 1;
        return { scrollableY, scrollableX, scrollHeight: se?.scrollHeight || 0, clientHeight: se?.clientHeight || 0 };
      }

      function countVisualLinesWithClone(el) {
        if (!el) return 0;
        const row = el.parentElement || el;
        const s = getComputedStyle(el);
        const r = getComputedStyle(row);
        const clone = document.createElement('div');
        clone.style.position = 'absolute';
        clone.style.left = '-99999px';
        clone.style.top = '-99999px';
        clone.style.whiteSpace = 'pre-wrap';
        clone.style.wordBreak = 'break-word';
        clone.style.overflowWrap = 'break-word';
        clone.style.fontFamily = s.fontFamily;
        clone.style.fontWeight = s.fontWeight;
        clone.style.fontStyle = s.fontStyle;
        clone.style.fontVariant = s.fontVariant;
        clone.style.letterSpacing = s.letterSpacing;
        clone.style.textTransform = s.textTransform;
        clone.style.textAlign = s.textAlign;
        clone.style.lineHeight = s.lineHeight;
        clone.style.fontSize = s.fontSize;
        // use row's inner content width
        const rr = row.getBoundingClientRect();
        const padL = parseFloat(r.paddingLeft || '0') || 0;
        const padR = parseFloat(r.paddingRight || '0') || 0;
        clone.style.width = Math.max(1, rr.width - padL - padR) + 'px';
        // copy text
        const text = (el.textContent || '').replace(/\s+$/,'');
        clone.textContent = text;
        document.body.appendChild(clone);
        const rects = [];
        // sample per character to get line tops reliably
        for (let i = 0; i < text.length; i += 1) {
          const range = document.createRange();
          try {
            range.setStart(clone.firstChild || clone, i);
            range.setEnd(clone.firstChild || clone, i + 1);
            const r = range.getBoundingClientRect();
            if (r && r.width > 0 && r.height > 0) rects.push(r);
          } catch {}
        }
        const tops = rects.map((r) => r.top).sort((a,b)=>a-b);
        // cluster by proximity (<= 2px) to avoid subpixel / font jitter
        let clusters = 0;
        let last = -1e9;
        for (const t of tops) {
          if (t - last > 2) {
            clusters += 1;
            last = t;
          }
        }
        const uniqueTops = clusters;
        clone.remove();
        return uniqueTops || 0;
      }

      return ids.map((id) => {
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

        const textContent = (element.textContent || '').trim();
        const explicitLines = textContent.length ? Math.max(1, textContent.split(/\r?\n/).length) : 0;

        const container = element.parentElement || element;
        const containerStyle = window.getComputedStyle(container);
        const containerRect = container.getBoundingClientRect();
        const elementRect = element.getBoundingClientRect();
        const padL = parseFloat(containerStyle.paddingLeft) || 0;
        const padR = parseFloat(containerStyle.paddingRight) || 0;
        const contentLeft = containerRect.left + padL;
        const contentRight = containerRect.left + containerRect.width - padR;
        const contentWidth = Math.max(0, contentRight - contentLeft);

        let maxLineWidth = elementRect.width;
        if (textContent.length) {
          const range = document.createRange();
          range.selectNodeContents(element);
          const rects = Array.from(range.getClientRects()).filter((rect) => rect.width > 1 && rect.height > 1);
          if (rects.length > 0) {
            maxLineWidth = Math.max(...rects.map((rect) => rect.width));
          }
        }
        const widthCoverage = contentWidth > 0 ? maxLineWidth / contentWidth : 0;

        const leftGutter = Math.max(0, elementRect.left - contentLeft);
        const rightGutter = Math.max(0, contentRight - (elementRect.left + elementRect.width));
        const paddingTop = parseFloat(style.paddingTop) || 0;
        const paddingBottom = parseFloat(style.paddingBottom) || 0;
        const contentHeight = Math.max(0, element.scrollHeight - paddingTop - paddingBottom);

        let lines = lineHeightPx > 0 ? contentHeight / lineHeightPx : 0;
        if (explicitLines >= 2) {
          lines = explicitLines;
        }

        const rects = Array.from(element.getClientRects()).filter((rect) => rect.width > 1 && rect.height > 1);
        // robust line counting using off-screen clone to avoid single-rect merging cases
        const uniqueLines = countVisualLinesWithClone(element);
        const parentHeightPx = element.parentElement?.getBoundingClientRect().height || 0;
        const occupancy = parentHeightPx > 0
          ? ((explicitLines > 0 ? explicitLines : lines) * lineHeightPx) / parentHeightPx
          : 0;

        const scroll = isPageScrollable();
        const stageEl = document.querySelector('.stage__lyrics');
        const stageRect = stageEl ? (stageEl as HTMLElement).getBoundingClientRect() : ({ height: 0 } as DOMRect);
        const viewportH = document.documentElement.clientHeight;
        const stageVsViewport = { stageHeightPx: (stageRect as any).height || 0, viewportHeightPx: viewportH };

        return {
          id,
          present: true,
          text: textContent,
          explicitLines,
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
          parentHeightPx,
          rootHeightPx: (element.closest('.stage__lyrics') as HTMLElement | null)?.getBoundingClientRect().height || 0,
          bottomOverflowPx: Math.max(0, (element.getBoundingClientRect().bottom) - ((element.parentElement?.getBoundingClientRect().bottom) || 0)),
          occupancy,
          pageScrollableY: scroll.scrollableY,
          pageScrollDims: { scrollHeight: scroll.scrollHeight, clientHeight: scroll.clientHeight },
          stageVsViewport,
        };
      });
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
          // Ensure the server reports the layout before loading the page
          await waitForServerLayout(request, scenario.layout);
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
        if (process.env.TRACE_FIT === '1') {
          await page.addInitScript(() => {
            (window as any).PRESENTER_STAGE_TEST_CONFIG = { traceFit: true };
          });
        }
        await page.goto(new URL('/stage', `${stageBaseUrl}/`).toString(), {
          waitUntil: 'domcontentloaded',
        });
        await page.waitForFunction(
          (expected) => {
            const bodyCode = document.body.getAttribute('data-layout-code');
            // @ts-ignore
            const winCode = (window.__presenterStageLayout || '').toString();
            return bodyCode === expected || winCode === expected;
          },
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
          const minWidth = Math.floor(widths.viewportWidth * 0.995);
          expect(widths.bodyWidth).toBeGreaterThanOrEqual(minWidth);
          expect(widths.stageBodyWidth).toBeGreaterThanOrEqual(minWidth);
          expect(widths.lyricsWidth).toBeGreaterThanOrEqual(Math.floor(widths.viewportWidth * 0.99));
          await test.info().attach('stage-screenshot', { body: await page.screenshot({ fullPage: false }), contentType: 'image/png' });
        }

          const metrics = await collectMetrics(page, scenario.elements);
          if (process.env.TRACE_FIT === '1') {
            const fitLog = await page.evaluate(() => {
              const log = (window as any).__presenterStageFitLog || [];
              (window as any).__presenterStageFitLog = [];
              return log;
            });
            if (Array.isArray(fitLog) && fitLog.length > 0) {
              await test.info().attach('fit-log', {
                contentType: 'application/json',
                body: JSON.stringify({ viewport, scenario: scenario.layout, slide: slide.label, fitLog }),
              });
            }
          }
          await test.info().attach('metrics', {
            contentType: 'application/json',
            body: JSON.stringify({ viewport, scenario: scenario.layout, slide: slide.label, metrics }),
          });

          for (const metric of metrics) {
            if (!metric.present || !metric.text) {
              continue;
            }
            // page must not be vertically scrollable
            expect(metric.pageScrollableY, `page must not scroll vertically (${scenario.layout} / ${slide.label})`).toBeFalsy();
            expect(Math.abs(metric.stageVsViewport.stageHeightPx - metric.stageVsViewport.viewportHeightPx), `stage height should match viewport (${scenario.layout} / ${slide.label})`).toBeLessThanOrEqual(1);
            // Line count check uses scrollHeight/lineHeight (robust across engines)
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
              if (metric.text) {
                const normalizedLen = metric.text.replace(/\s+/g, '').length;
                const enforceWidth = normalizedLen >= 10;
                const minCoverage = metric.explicitLines >= 2 ? 0.8 : 0.6;
                if (enforceWidth) {
                  expect(
                    metric.widthCoverage,
                    `width coverage for ${scenario.layout} / ${slide.label} / ${metric.id}`,
                  ).toBeGreaterThanOrEqual(minCoverage);
                }
                if (metric.explicitLines >= 2) {
                  expect(
                    metric.occupancy,
                    `occupancy for ${scenario.layout} / ${slide.label} / ${metric.id}`,
                  ).toBeGreaterThanOrEqual(0.35);
                }
              }
              const maxGutterPx = 6;
              expect(metric.leftGutterPx).toBeLessThanOrEqual(maxGutterPx);
              expect(metric.rightGutterPx).toBeLessThanOrEqual(maxGutterPx);
              expect(metric.bodyPaddingLeftPx).toBeLessThanOrEqual(6);
              expect(metric.bodyPaddingRightPx).toBeLessThanOrEqual(6);

              // Equal split check (SNV: current/next halves remain stable)
              const next = metrics.find((m) => m.id === 'next-text');
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

            if (isCurrent && scenario.layout === 'worship-pp' && isRetina) {
              if (metric.text) {
                const normalizedLen = metric.text.replace(/\s+/g, '').length;
                const enforceWidth = normalizedLen >= 10;
                const minCoverage = metric.explicitLines >= 2 ? 0.78 : 0.6;
                if (enforceWidth) {
                  expect(
                    metric.widthCoverage,
                    `width coverage for ${scenario.layout} / ${slide.label} / ${metric.id}`,
                  ).toBeGreaterThanOrEqual(minCoverage);
                }
                if (metric.explicitLines >= 2) {
                  expect(
                    metric.occupancy,
                    `occupancy for ${scenario.layout} / ${slide.label} / ${metric.id}`,
                  ).toBeGreaterThanOrEqual(0.24);
                }
              }
              const next = metrics.find((m) => m.id === 'next-main');
              if (next && next.present) {
                expect(metric.bottomOverflowPx).toBeLessThanOrEqual(1);
                expect(next.bottomOverflowPx).toBeLessThanOrEqual(1);
              }
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
