import { test, expect } from '@playwright/test';
import { deriveTestConfig, refreshDevData, startTestServer, stopServer, type ServerHandle } from './support';

const RUN = process.env.STAGE_NEGATIVE === '1';
const EXPLICIT_STAGE_URL = process.env.STAGE_URL?.replace(/\/$/, '');

const runOrSkip = RUN ? test.describe : test.describe.skip;

runOrSkip('Negative: undersized two-line current on Retina should fail', () => {
  let server: ServerHandle | undefined;
  let baseURL = EXPLICIT_STAGE_URL as string | undefined;
  let dbUrl: string;
  let port: number;

  test.beforeAll(async ({}, testInfo) => {
    if (!baseURL) {
      const cfg = deriveTestConfig(testInfo);
      port = cfg.port;
      dbUrl = cfg.dbUrl;
      baseURL = cfg.baseURL;
      await refreshDevData(dbUrl);
      server = await startTestServer(port, dbUrl, cfg.oscPort);
    }
  });

  test.afterAll(async () => { await stopServer(server); server = undefined; });

  test('Nehladam svoje (Nezáleží…) at 2880x1800 violates occupancy', async ({ page, request }) => {
    test.info().annotations.push({ type: 'negative', description: 'Retina two-line should be large by height' });
    if (!baseURL) throw new Error('baseURL not initialised');
    await page.setViewportSize({ width: 2880, height: 1800 });
    // Set SNV layout and target the known slide
    await request.post(new URL('/stage/layout', `${baseURL}/`).toString(), { data: { code: 'worship-snv' } });
    const search = await request.get(new URL('/search?query=Nehl', `${baseURL}/`).toString());
    const results = (await search.json()) as any[];
    const pres = results.find(r => r.kind === 'presentation' && /Nehl.+svoje/.test(r.presentationName));
    const detail = await (await request.get(new URL(`/presentations/${pres.presentationId}`, `${baseURL}/`).toString())).json();
    const slides: any[] = detail.presentation.slides;
    const idx = slides.findIndex(s => (s.content?.main?.value || '').includes('Nezáleží už na tom čo chcem ja'));
    const currentSlideId = slides[idx]?.id;
    const nextSlideId = slides[idx + 1]?.id;
    await request.post(new URL('/stage/state', `${baseURL}/`).toString(), { data: { presentationId: pres.presentationId, currentSlideId, nextSlideId } });
    await page.goto(new URL('/stage', `${baseURL}/`).toString(), { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => (window as any).__presenterStageLayout === 'worship-snv');
    await page.waitForTimeout(150);
    const m = await page.evaluate(() => {
      const el = document.getElementById('current-text');
      const row = el?.parentElement as HTMLElement | null;
      const fs = el ? parseFloat(getComputedStyle(el).fontSize) || 0 : 0;
      let lh = el ? parseFloat(getComputedStyle(el).lineHeight) : 0;
      if (!Number.isFinite(lh) || lh <= 0) lh = fs * 1.12;
      const text = (el?.textContent || '').trim();
      const explicitLines = text.length ? Math.max(1, text.split(/\r?\n/).length) : 0;
      let lines = el && lh > 0 ? (el.scrollHeight / lh) : 0;
      if (explicitLines >= 2) {
        lines = explicitLines;
      }
      const rowH = row ? row.getBoundingClientRect().height : 0;
      const occupancy = rowH > 0 ? ((explicitLines > 0 ? explicitLines : lines) * lh) / rowH : 0;
      return { fs, lh, lines, explicitLines, rowH, occupancy, text };
    });
    await test.info().attach('live-metrics', { contentType: 'application/json', body: JSON.stringify(m) });
    // Intentional failing assertion to demonstrate detection of undersized two-line renders
    expect(m.occupancy).toBeGreaterThanOrEqual(0.35);
  });
});
