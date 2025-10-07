#!/usr/bin/env node
import { chromium, request } from '@playwright/test';
import fs from 'node:fs/promises';

const manifestPath = process.env.MANIFEST || `${process.env.HOME}/.local/share/presenter-demos/manifests/presenter-dev1.json`;
const manifest = JSON.parse(await fs.readFile(manifestPath, 'utf8'));
const base = `http://127.0.0.1:${manifest.port}`;

// Resolve the slide id for the target presentation + text
const api = await request.newContext();
const searchRes = await api.get(`${base}/search?query=Nehl%CC%8Cada%CC%81m%20svoje`);
if (!searchRes.ok()) throw new Error('search failed');
const results = await searchRes.json();
const pres = results.find(r => r.kind === 'presentation' && r.presentationName === 'Nehľadám svoje');
if (!pres) throw new Error('presentation not found');
const detail = await (await api.get(`${base}/presentations/${pres.presentationId}`)).json();
const slides = detail.presentation.slides;
const target = slides.find(s => (s.content?.main?.value || '').includes('Nezáleží už na tom čo chcem ja'));
if (!target) throw new Error('target slide not found');
const nextIdx = Math.min(slides.length - 1, slides.findIndex(s => s.id === target.id) + 1);
const nextId = slides[nextIdx]?.id;

// Set layout and state
await api.post(`${base}/stage/layout`, { data: { code: 'worship-snv' } });
await api.post(`${base}/stage/state`, { data: { presentationId: pres.presentationId, currentSlideId: target.id, nextSlideId: nextId } });

// Launch browser and measure
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 2880, height: 1800 } });
const page = await context.newPage();
await page.goto(`${base}/stage`, { waitUntil: 'domcontentloaded' });
await page.waitForFunction(() => window.__presenterStageLayout === 'worship-snv');
await page.waitForTimeout(150);

const widths = await page.evaluate(() => {
  const stageBody = document.querySelector('main.stage__body');
  const lyrics = document.querySelector('.stage__lyrics');
  const bodyRect = document.body.getBoundingClientRect();
  const stageRect = stageBody ? stageBody.getBoundingClientRect() : { width: 0 };
  const lyricsRect = lyrics ? lyrics.getBoundingClientRect() : { width: 0 };
  const viewportWidth = document.documentElement.clientWidth;
  return {
    viewportWidth,
    bodyWidth: bodyRect.width,
    stageBodyWidth: stageRect.width,
    lyricsWidth: lyricsRect.width,
  };
});

const shotPath = `artifacts/snv-nezalezi-retina.png`;
await page.screenshot({ path: shotPath });
console.log(JSON.stringify({ base, presId: pres.presentationId, slideId: target.id, widths, screenshot: shotPath }, null, 2));

await browser.close();
await api.dispose();
