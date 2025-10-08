#!/usr/bin/env node
import { chromium } from 'playwright';
import fs from 'node:fs/promises';

const target = (process.env.STAGE_URL || 'http://127.0.0.1:18564/stage').replace(/\/$/, '');
const viewport = { width: 2880, height: 1800 };

function ts() { return new Date().toISOString().replace(/[:.]/g, '-'); }

function overlayScript() {
  const px = (n) => `${n}px`;
  const add = (parent, style = {}) => {
    const el = document.createElement('div');
    Object.assign(el.style, style);
    parent.appendChild(el);
    return el;
  };
  const outline = (rect, color = 'rgba(255,0,0,0.8)', thickness = 2) => {
    return add(container, {
      position: 'absolute',
      left: px(rect.left + window.scrollX),
      top: px(rect.top + window.scrollY),
      width: px(rect.width),
      height: px(rect.height),
      border: `${thickness}px dashed ${color}`,
      pointerEvents: 'none',
      boxSizing: 'border-box',
    });
  };

  const container = document.createElement('div');
  container.setAttribute('data-mark-overlay', '1');
  Object.assign(container.style, {
    position: 'absolute',
    left: '0',
    top: '0',
    width: '100%',
    height: '100%',
    pointerEvents: 'none',
    zIndex: 999999,
  });
  document.body.appendChild(container);

  const q = (sel) => document.querySelector(sel);
  const getRect = (el) => el ? el.getBoundingClientRect() : { left:0, top:0, width:0, height:0, right:0, bottom:0 };

  const bodyRect = document.body.getBoundingClientRect();
  outline(bodyRect, 'rgba(255,255,255,0.6)', 3); // body

  const lyrics = q('.stage__lyrics');
  const curRow = q('.stage__lyrics-current');
  const nextRow = q('.stage__lyrics-next');
  const curText = q('#current-text,#current-main');
  const nextText = q('#next-text,#next-main');

  const lyricsRect = getRect(lyrics);
  const curRowRect = getRect(curRow);
  const nextRowRect = getRect(nextRow);
  const curTextRect = getRect(curText);
  const nextTextRect = getRect(nextText);

  // Outline main containers
  outline(lyricsRect, 'rgba(0,200,255,0.9)', 3);
  outline(curRowRect, 'rgba(0,255,0,0.9)', 3);
  outline(nextRowRect, 'rgba(0,150,255,0.9)', 3);
  outline(curTextRect, 'rgba(255,0,0,0.9)', 2);
  outline(nextTextRect, 'rgba(255,80,0,0.9)', 2);

  // Content left/right rulers (accounting for row padding)
  const drawContentRulers = (rowEl, color) => {
    if (!rowEl) return;
    const rs = getComputedStyle(rowEl);
    const rr = rowEl.getBoundingClientRect();
    const padL = parseFloat(rs.paddingLeft) || 0;
    const padR = parseFloat(rs.paddingRight) || 0;
    const contentLeft = rr.left + padL;
    const contentRight = rr.left + rr.width - padR;
    // left ruler
    add(container, {
      position: 'absolute',
      left: px(contentLeft + window.scrollX),
      top: px(rr.top + window.scrollY),
      width: '2px',
      height: px(rr.height),
      background: color,
    });
    // right ruler
    add(container, {
      position: 'absolute',
      left: px(contentRight + window.scrollX - 2),
      top: px(rr.top + window.scrollY),
      width: '2px',
      height: px(rr.height),
      background: color,
    });
  };
  drawContentRulers(curRow, 'rgba(0,255,0,0.9)');
  drawContentRulers(nextRow, 'rgba(0,150,255,0.9)');

  return true;
}

(async () => {
  await fs.mkdir('artifacts', { recursive: true });
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport });
  const page = await ctx.newPage();
  await page.goto(target, { waitUntil: 'domcontentloaded' });
  await page.evaluate(overlayScript);
  const outBase = `artifacts/mark-live-${ts()}`;
  await page.screenshot({ path: `${outBase}.png` });
  console.log(JSON.stringify({ ok: true, outPng: `${outBase}.png`, target }));
  await browser.close();
})();

