import { test, expect } from '@playwright/test';
import type { APIRequestContext, Page, TestInfo } from '@playwright/test';
import { deriveTestConfig, refreshDevData, startTestServer, stopServer, type ServerHandle } from './support';

const EXPLICIT_STAGE_URL = process.env.STAGE_URL?.replace(/\/$/, '');
const AUDIT_LIBS = (process.env.AUDIT_LIBS || '').split(',').map(s => s.trim()).filter(Boolean);
const MAX_PRESENTATIONS = Number(process.env.AUDIT_MAX_PRESENTATIONS || '0');
const MAX_SLIDES = Number(process.env.AUDIT_MAX_SLIDES || '0');
const VIEWPORT = { width: 2880, height: 1800 } as const;

let baseURL: string | undefined = EXPLICIT_STAGE_URL;
let server: ServerHandle | undefined;
let dbUrl: string;
let port: number;

test.describe.configure({ mode: 'serial', timeout: 2 * 60 * 60 * 1000 });

const shouldRun = AUDIT_LIBS.length > 0;

const describeFn = shouldRun ? test.describe : test.describe.skip;

describeFn('Stage Audit (SNV, Retina, width coverage, equal split)', () => {
  test.beforeAll(async ({}, testInfo: TestInfo) => {
    if (!baseURL) {
      const cfg = deriveTestConfig(testInfo);
      port = cfg.port;
      dbUrl = cfg.dbUrl;
      baseURL = cfg.baseURL;
      await refreshDevData(dbUrl);
      server = await startTestServer(port, dbUrl, cfg.oscPort);
    }
  });

  test.afterAll(async () => {
    await stopServer(server);
    server = undefined;
  });

  async function getJson<T = any>(request: APIRequestContext, path: string): Promise<T> {
    if (!baseURL) throw new Error('baseURL not initialised');
    const res = await request.get(new URL(path, `${baseURL}/`).toString());
    expect(res.ok()).toBeTruthy();
    return (await res.json()) as T;
  }

  async function postJson(request: APIRequestContext, path: string, body: Record<string, unknown>) {
    if (!baseURL) throw new Error('baseURL not initialised');
    const res = await request.post(new URL(path, `${baseURL}/`).toString(), { data: body });
    expect(res.ok(), `POST ${path} failed (${res.status()})`).toBeTruthy();
  }

  async function waitForServerLayout(
    request: APIRequestContext,
    expected: string,
    timeoutMs = 10000,
    intervalMs = 200,
  ) {
    const start = Date.now();
    for (;;) {
      const res = await request.get(new URL('/stage/layout', `${baseURL}/`).toString());
      expect(res.ok()).toBeTruthy();
      const js: any = await res.json();
      const code = (js?.code || js?.layoutCode || js?.layout_code || '').toString();
      if (code === expected) return;
      if (Date.now() - start > timeoutMs) {
        throw new Error(`Server layout did not switch to '${expected}' in ${timeoutMs}ms`);
      }
      await new Promise(r => setTimeout(r, intervalMs));
    }
  }

  async function listPresentations(request: APIRequestContext, libraryName: string) {
    const items = await getJson<any[]>(request, `/search?query=${encodeURIComponent(libraryName)}`);
    return items.filter(it => it?.kind === 'presentation').map(it => ({ id: it.presentationId as string, name: it.presentationName as string }));
  }

  async function collectMetrics(page: Page) {
    return page.evaluate(() => {
      function isPageScrollable() {
        const se = document.scrollingElement || document.documentElement;
        return {
          scrollableY: (se?.scrollHeight || 0) > (se?.clientHeight || 0) + 1,
          scrollHeight: se?.scrollHeight || 0,
          clientHeight: se?.clientHeight || 0,
        };
      }
      function countVisualLinesWithClone(el: HTMLElement | null) {
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
        const rr = row.getBoundingClientRect();
        const padL = parseFloat(r.paddingLeft || '0') || 0;
        const padR = parseFloat(r.paddingRight || '0') || 0;
        clone.style.width = Math.max(1, rr.width - padL - padR) + 'px';
        const text = (el.textContent || '').replace(/\s+$/,'');
        clone.textContent = text;
        document.body.appendChild(clone);
        const rects: DOMRect[] = [] as any;
        for (let i = 0; i < text.length; i += 1) {
          const range = document.createRange();
          try {
            range.setStart(clone.firstChild as any, i);
            range.setEnd(clone.firstChild as any, i + 1);
            const r = range.getBoundingClientRect();
            if (r && r.width > 0 && r.height > 0) rects.push(r);
          } catch {}
        }
        const tops = rects.map((r: DOMRect) => r.top).sort((a,b)=>a-b);
        let clusters = 0;
        let last = -1e9;
        for (const t of tops) {
          if (t - last > 2) { clusters += 1; last = t; }
        }
        const uniqueTops = clusters;
        clone.remove();
        return uniqueTops || 0;
      }
      const el = document.getElementById('current-text') || document.getElementById('current-main');
      const next = document.getElementById('next-text') || document.getElementById('next-main');
      const lyrics = document.querySelector('.stage__lyrics');
      const curRow = document.querySelector('.stage__lyrics-current');
      const nextRow = document.querySelector('.stage__lyrics-next');
      const getRect = (e: Element | null) => (e ? (e as HTMLElement).getBoundingClientRect() : ({ left:0, top:0, width:0, height:0, right:0, bottom:0 } as DOMRect));
      const style = el ? getComputedStyle(el as HTMLElement) : ({} as CSSStyleDeclaration);
      const fs = parseFloat((style as any).fontSize || '0') || 0;
      let lh = parseFloat((style as any).lineHeight || '0');
      if (!Number.isFinite(lh) || lh <= 0) lh = fs * 1.12;
      const text = (el?.textContent || '').trim();
      const explicitLines = text.length ? Math.max(1, text.split(/\r?\n/).length) : 0;
      const rect = el ? getRect(el) : ({} as DOMRect);
      const container = (el?.parentElement || null) as HTMLElement | null;
      const containerRect = getRect(container);
      const containerStyle = container ? getComputedStyle(container) : ({} as CSSStyleDeclaration);
      const padL = parseFloat((containerStyle as any).paddingLeft || '0') || 0;
      const padR = parseFloat((containerStyle as any).paddingRight || '0') || 0;
      const contentLeft = containerRect.left + padL;
      const contentRight = containerRect.left + containerRect.width - padR;
      const contentWidth = Math.max(0, contentRight - contentLeft);
      let maxLineWidth = rect.width;
      if (text.length) {
        const range = document.createRange();
        range.selectNodeContents(el as Node);
        const rects = Array.from(range.getClientRects()).filter((r) => r.width > 1 && r.height > 1);
        if (rects.length > 0) {
          maxLineWidth = Math.max(...rects.map((r) => r.width));
        }
      }
      const widthCoverage = contentWidth > 0 ? maxLineWidth / contentWidth : 0;
      const leftGutter = Math.max(0, rect.left - contentLeft);
      const rightGutter = Math.max(0, contentRight - (rect.left + rect.width));
      const rawLines = lh > 0 ? Math.max(0, (el as HTMLElement)?.scrollHeight - (parseFloat((style as any).paddingTop || '0')||0) - (parseFloat((style as any).paddingBottom || '0')||0)) / lh : 0;
      let lines = rawLines;
      if (explicitLines > 0 && lines < explicitLines) {
        lines = explicitLines;
      }
      const uniqueLines = countVisualLinesWithClone(el as HTMLElement);
      const lyricsRect = getRect(lyrics);
      const curRowRect = getRect(curRow);
      const nextRowRect = getRect(nextRow);
      const occupancy = curRowRect.height > 0 ? ((explicitLines > 0 ? explicitLines : lines) * lh) / curRowRect.height : 0;
      const scroll = isPageScrollable();
      const viewportH = document.documentElement.clientHeight;
      return {
        layout: (window as any).__presenterStageLayout || document.body.getAttribute('data-layout-code'),
        fontSizePx: fs,
        lineHeightPx: lh,
        lines,
        explicitLines,
        text,
        uniqueLines,
        widthCoverage,
        leftGutterPx: leftGutter,
        rightGutterPx: rightGutter,
        lyricsWidth: lyricsRect.width,
        curRowWidth: curRowRect.width,
        nextRowWidth: nextRowRect.width,
        curRowHeight: curRowRect.height,
        nextRowHeight: nextRowRect.height,
        occupancy,
        pageScrollableY: scroll.scrollableY,
        pageScrollDims: { scrollHeight: scroll.scrollHeight, clientHeight: scroll.clientHeight },
        stageVsViewport: { stageHeightPx: lyricsRect.height, viewportHeightPx: viewportH },
      };
    });
  }

  for (const library of AUDIT_LIBS) {
    test(`audit library: ${library}`, async ({ page, request }, testInfo) => {
      testInfo.annotations.push({ type: 'library', description: library });
      if (!baseURL) throw new Error('baseURL not initialised');
      // Configure viewport for this test
      await page.setViewportSize(VIEWPORT as any);
      if (process.env.TRACE_FIT === '1') {
        await page.addInitScript(() => {
          (window as any).PRESENTER_STAGE_TEST_CONFIG = { traceFit: true };
        });
      }

      // Stage layout: SNV only for now
      await postJson(request, '/stage/layout', { code: 'worship-snv' });
      await waitForServerLayout(request, 'worship-snv');

      const list = await listPresentations(request, library);
      const presList = MAX_PRESENTATIONS > 0 ? list.slice(0, MAX_PRESENTATIONS) : list;
      for (const { id, name } of presList) {
        const detail = await getJson<any>(request, `/presentations/${id}`);
        const slides: any[] = detail?.presentation?.slides ?? [];
        const slidesToCheck = MAX_SLIDES > 0 ? slides.slice(0, MAX_SLIDES) : slides;
        for (let i = 0; i < slidesToCheck.length; i += 1) {
          const currentSlideId = slidesToCheck[i]?.id;
          const nextSlideId = slidesToCheck[i + 1]?.id;
          if (!currentSlideId) continue;
          await postJson(request, '/stage/state', { presentationId: id, currentSlideId, nextSlideId });
          await page.goto(new URL('/stage', `${baseURL}/`).toString(), { waitUntil: 'domcontentloaded' });
          await page.waitForFunction((expected) => {
            const bodyCode = document.body.getAttribute('data-layout-code');
            // @ts-ignore
            const winCode = (window.__presenterStageLayout || '').toString();
            return bodyCode === expected || winCode === expected;
          }, 'worship-snv');
          await page.waitForTimeout(125);
          const m = await collectMetrics(page);
          // Attach minimal context for traceability
          await testInfo.attach('metrics', { contentType: 'application/json', body: JSON.stringify({ library, name, slideIndex: i, metrics: m }) });
          if (process.env.TRACE_FIT === '1') {
            const fitLog = await page.evaluate(() => {
              const log = (window as any).__presenterStageFitLog || [];
              (window as any).__presenterStageFitLog = [];
              return log;
            });
            await testInfo.attach('fit-log', { contentType: 'application/json', body: JSON.stringify({ library, name, slideIndex: i, log: fitLog }) });
          }
          // Skip assertions on blank slides (no visible current text)
          if (m.text && m.text.trim().length > 0) {
            // page must not be scrollable, and stage height must == viewport height
            expect.soft(m.pageScrollableY, `${library} / ${name} [${i}] page scrollable`).toBeFalsy();
            expect.soft(Math.abs(m.stageVsViewport.stageHeightPx - m.stageVsViewport.viewportHeightPx), `${library} / ${name} [${i}] stage height vs viewport`).toBeLessThanOrEqual(1);
            // Two-line cap only (≤ 2 lines) when the slide has ≤ 2 explicit lines
            if ((m.explicitLines || 0) <= 2) {
              expect(m.lines, `${library} / ${name} [${i}] lines`).toBeLessThanOrEqual(2.02);
            }
          }
        }
      }
    });
  }
});
