#!/usr/bin/env node
import { chromium } from 'playwright';
import fs from 'node:fs/promises';

const BASE = process.env.STAGE_URL || 'http://127.0.0.1:18564';
const LIBS = (process.env.AUDIT_LIBS || 'BOHATY MUSIC,NEW LEVEL').split(',').map(s => s.trim()).filter(Boolean);
const VIEWPORT = { width: 2880, height: 1800 };
const OUT_DIR = 'artifacts';

async function getJson(path) {
  const res = await fetch(new URL(path, `${BASE}/`).toString());
  if (!res.ok) throw new Error(`GET ${path} -> ${res.status}`);
  return res.json();
}

async function postJson(path, body) {
  const res = await fetch(new URL(path, `${BASE}/`).toString(), { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (!res.ok) throw new Error(`POST ${path} -> ${res.status}`);
}

function ts() { return new Date().toISOString().replace(/[:.]/g, '-'); }

async function* listSlides(libraryName) {
  const items = await getJson(`/search?query=${encodeURIComponent(libraryName)}`);
  const pres = items.filter(it => it?.kind === 'presentation').map(it => ({ id: it.presentationId, name: it.presentationName }));
  for (const p of pres) {
    const detail = await getJson(`/presentations/${p.id}`);
    const slides = detail?.presentation?.slides ?? [];
    for (let i = 0; i < slides.length; i += 1) {
      const currentId = slides[i]?.id;
      const nextId = slides[i + 1]?.id;
      if (!currentId) continue;
      yield { library: libraryName, presentation: p.name, presentationId: p.id, index: i, currentId, nextId };
    }
  }
}

async function gatherMetrics(page) {
  return page.evaluate(() => {
    const q = (sel) => document.querySelector(sel);
    const getRect = (el) => el ? el.getBoundingClientRect() : ({ left:0, top:0, width:0, height:0, right:0, bottom:0 });
    const el = q('#current-text') || q('#current-main');
    const next = q('#next-text') || q('#next-main');
    const lyrics = q('.stage__lyrics');
    const curRow = q('.stage__lyrics-current');
    const nextRow = q('.stage__lyrics-next');
    const style = el ? getComputedStyle(el) : null;
    const fs = style ? parseFloat(style.fontSize) || 0 : 0;
    let lh = style ? parseFloat(style.lineHeight) : 0;
    if (!Number.isFinite(lh) || lh <= 0) lh = fs * 1.12;
    const text = (el?.textContent || '').trim();
    const explicitLines = text.length ? Math.max(1, text.split(/\r?\n/).length) : 0;
    const er = getRect(el);
    const cont = el?.parentElement || null;
    const crs = cont ? getComputedStyle(cont) : null;
    const cr = cont ? getRect(cont) : ({ left:0, width:0, height:0 });
    const padL = crs ? (parseFloat(crs.paddingLeft) || 0) : 0;
    const padR = crs ? (parseFloat(crs.paddingRight) || 0) : 0;
    const contentLeft = cr.left + padL;
    const contentRight = cr.left + cr.width - padR;
    const contentWidth = Math.max(0, contentRight - contentLeft);
    let maxLineWidth = er.width;
    if (text.length) {
      const range = document.createRange();
      if (el) { range.selectNodeContents(el); }
      const rects = Array.from(range.getClientRects()).filter((r) => r.width > 1 && r.height > 1);
      if (rects.length > 0) {
        maxLineWidth = Math.max(...rects.map((r) => r.width));
      }
    }
    const widthCoverage = contentWidth > 0 ? maxLineWidth / contentWidth : 0;
    const leftGutter = Math.max(0, er.left - contentLeft);
    const rightGutter = Math.max(0, contentRight - (er.left + er.width));
    let lines = lh > 0 ? Math.max(0, (el ? el.scrollHeight : 0) / lh) : 0;
    if (explicitLines >= 2) {
      lines = explicitLines;
    }
    const lr = getRect(lyrics);
    const rr = getRect(curRow);
    const nr = getRect(nextRow);
    const occupancy = rr.height > 0 ? ((explicitLines > 0 ? explicitLines : lines) * lh) / rr.height : 0;
    return {
      layout: (window).__presenterStageLayout || document.body.getAttribute('data-layout-code'),
      text,
      explicitLines,
      fontSizePx: fs,
      lineHeightPx: lh,
      lines,
      widthCoverage,
      leftGutterPx: leftGutter,
      rightGutterPx: rightGutter,
      lyricsWidth: lr.width,
      curRowWidth: rr.width,
      nextRowWidth: nr.width,
      curRowHeight: rr.height,
      nextRowHeight: nr.height,
      occupancy,
    };
  });
}

async function main() {
  await fs.mkdir(OUT_DIR, { recursive: true });
  const outBase = `${OUT_DIR}/audit-${ts()}`;
  const outJsonl = `${outBase}.jsonl`;
  const outSummary = `${outBase}-summary.json`;
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: VIEWPORT });
  const page = await ctx.newPage();
  const summary = { base: BASE, viewport: VIEWPORT, startedAt: new Date().toISOString(), totals: { slides: 0, checked: 0, failures: 0 }, libraries: {} };

  await postJson('/stage/layout', { code: 'worship-snv' });

  for (const lib of LIBS) {
    summary.libraries[lib] = { slides: 0, checked: 0, failures: 0, failuresByPresentation: {} };
    for await (const item of listSlides(lib)) {
      summary.totals.slides += 1;
      summary.libraries[lib].slides += 1;
      await postJson('/stage/state', { presentationId: item.presentationId, currentSlideId: item.currentId, nextSlideId: item.nextId });
      await page.goto(new URL('/stage', `${BASE}/`).toString(), { waitUntil: 'domcontentloaded' });
      await page.waitForFunction(() => (window).__presenterStageLayout === 'worship-snv', { timeout: 15000 });
      await page.waitForTimeout(100);
      const m = await gatherMetrics(page);

      // Evaluate expectations only when there is text
      let ok = true;
      const errs = [];
      if (m.text && m.text.length > 0) {
        if (!(m.lines <= 2.2)) { ok = false; errs.push(`lines=${m.lines.toFixed(2)}`); }
        const normalizedLen = m.text.replace(/\s+/g, '').length;
        const enforceWidth = normalizedLen >= 10;
        const minCoverage = m.explicitLines >= 2 ? 0.8 : 0.6;
        if (enforceWidth && !(m.widthCoverage >= minCoverage)) { ok = false; errs.push(`widthCoverage=${(m.widthCoverage*100).toFixed(1)}%`); }
        if (!(m.curRowWidth >= m.lyricsWidth * 0.98)) { ok = false; errs.push(`curRowWidth=${m.curRowWidth} lyricsWidth=${m.lyricsWidth}`); }
        const share = (m.curRowHeight + m.nextRowHeight) > 0 ? m.curRowHeight / (m.curRowHeight + m.nextRowHeight) : 0.5;
        if (!(share >= 0.48 && share <= 0.52)) { ok = false; errs.push(`split=${(share*100).toFixed(1)}%`); }
        if (m.explicitLines >= 2) {
          if (!(m.occupancy >= 0.35)) { ok = false; errs.push(`occupancy=${(m.occupancy*100).toFixed(1)}%`); }
        }
      }

      const record = { ...item, metrics: m, ok, errs };
      await fs.appendFile(outJsonl, JSON.stringify(record) + '\n');
      if (!ok) {
        summary.totals.failures += 1;
        summary.libraries[lib].failures += 1;
        summary.libraries[lib].failuresByPresentation[item.presentation] = (summary.libraries[lib].failuresByPresentation[item.presentation] || 0) + 1;
      }
      summary.totals.checked += 1;
      summary.libraries[lib].checked += 1;
      if (summary.totals.checked % 100 === 0) {
        console.log(`[audit] ${summary.totals.checked} slides checked, failures=${summary.totals.failures}`);
      }
    }
  }

  summary.completedAt = new Date().toISOString();
  await fs.writeFile(outSummary, JSON.stringify(summary, null, 2));
  await browser.close();
  console.log(JSON.stringify({ ok: true, outJsonl, outSummary }, null, 2));
}

main().catch((err) => { console.error(err); process.exit(1); });

