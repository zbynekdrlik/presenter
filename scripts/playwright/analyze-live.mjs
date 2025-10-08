#!/usr/bin/env node
import { chromium } from 'playwright';
import fs from 'node:fs/promises';

const target = process.env.STAGE_URL || 'http://127.0.0.1:18564/stage';
const viewport = { width: 2880, height: 1800 };

function nowIso() { return new Date().toISOString().replace(/[:.]/g, '-'); }

function script() {
  const gather = () => {
    const q = (sel) => document.querySelector(sel);
    const qa = (sel) => Array.from(document.querySelectorAll(sel));
    const getRect = (el) => el ? el.getBoundingClientRect() : { left:0, top:0, width:0, height:0, right:0, bottom:0 };

    const layout = window.__presenterStageLayout || document.body.dataset.layoutCode || null;
    const bodyRect = document.body.getBoundingClientRect();

    const stageBody = q('main.stage__body');
    const stageLyrics = q('.stage__lyrics');
    const curRow = q('.stage__lyrics-current');
    const nextRow = q('.stage__lyrics-next');

    const stageBodyRect = getRect(stageBody);
    const lyricsRect = getRect(stageLyrics);
    const curRowRect = getRect(curRow);
    const nextRowRect = getRect(nextRow);

    // Determine split axis
    const dx = Math.abs(curRowRect.left - nextRowRect.left);
    const dy = Math.abs(curRowRect.top - nextRowRect.top);
    const splitAxis = dy > dx ? 'vertical' : 'horizontal';

    const currentIds = ['current-text', 'current-main'];
    const nextIds = ['next-text', 'next-main'];
    const ids = [...currentIds, ...nextIds];

    const computeMetrics = (element) => {
      if (!element) return null;
      const style = getComputedStyle(element);
      const fontSizePx = parseFloat(style.fontSize) || 0;
      let lineHeightPx = parseFloat(style.lineHeight);
      if (!Number.isFinite(lineHeightPx) || lineHeightPx <= 0) {
        lineHeightPx = fontSizePx * 1.12;
      }
      const container = element.parentElement || element;
      const cr = getComputedStyle(container);
      const containerRect = container.getBoundingClientRect();
      const elementRect = element.getBoundingClientRect();
      const padL = parseFloat(cr.paddingLeft) || 0;
      const padR = parseFloat(cr.paddingRight) || 0;
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
      const rects = Array.from(element.getClientRects()).filter(r => r.width > 1 && r.height > 1);
      const uniqueLines = Array.from(new Set(rects.map(r => r.top.toFixed(2)))).length;
      return {
        text: (element.textContent || '').trim(),
        fontSizePx,
        lineHeightPx,
        lines,
        uniqueLines,
        containerWidthPx: containerRect.width,
        elementWidthPx: elementRect.width,
        widthCoverage,
        leftGutterPx: leftGutter,
        rightGutterPx: rightGutter,
        parentHeightPx: containerRect.height,
        bottomOverflowPx: Math.max(0, (elementRect.bottom) - (containerRect.bottom)),
      };
    };

    const elements = ids.map(id => ({ id, el: document.getElementById(id) })).filter(x => x.el);
    const metrics = Object.fromEntries(elements.map(({id, el}) => [id, computeMetrics(el)]));

    const styleBody = getComputedStyle(document.body);

    return {
      targetHref: location.href,
      layout,
      viewportWidth: document.documentElement.clientWidth,
      viewportHeight: document.documentElement.clientHeight,
      body: { width: bodyRect.width, paddingLeftPx: parseFloat(styleBody.paddingLeft)||0, paddingRightPx: parseFloat(styleBody.paddingRight)||0 },
      stageBody: { width: stageBodyRect.width, height: stageBodyRect.height },
      lyrics: { width: lyricsRect.width, height: lyricsRect.height },
      rows: { current: { x: curRowRect.left, y: curRowRect.top, w: curRowRect.width, h: curRowRect.height }, next: { x: nextRowRect.left, y: nextRowRect.top, w: nextRowRect.width, h: nextRowRect.height } },
      splitAxis,
      metrics,
    };
  };
  return gather();
}

(async () => {
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  await page.goto(target, { waitUntil: 'domcontentloaded' });
  // allow scaler to settle without changing state
  await page.waitForTimeout(200);
  const data = await page.evaluate(script);
  const outBase = `artifacts/analyze-live-${nowIso()}`;
  await fs.writeFile(`${outBase}.json`, JSON.stringify(data, null, 2));
  await page.screenshot({ path: `${outBase}.png` });
  console.log(JSON.stringify({ ok: true, outJson: `${outBase}.json`, outPng: `${outBase}.png`, layout: data.layout, splitAxis: data.splitAxis, widths: { viewport: data.viewportWidth, body: data.body.width, stage: data.stageBody.width, lyrics: data.lyrics.width } }, null, 2));
  await browser.close();
})();
