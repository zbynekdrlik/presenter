// Standalone real-world video-playback verification against a running
// presenter instance with an active NDI source. Launches branded Chrome
// (not the default open-source Chromium, which lacks H.264) with Chrome's
// autoplay policy ENFORCED, matching real-user behaviour. Used when the
// in-repo Playwright tests can't reach the live NDI broadcaster (e.g.
// the test-server runs in a subprocess context without access).
//
// Usage:
//   node scripts/diag/verify_video_autoplay.mjs               # hits dev
//   PRESENTER_URL=http://10.77.9.205/stage node scripts/...   # any host
//
// Pre-reqs:
//   npm i  &&  npx playwright install chrome --with-deps
//
// Exit code 0 = video playing (paused=false AND currentTime > 0.1).
// Exit code 1 = failed; full diagnostic (samples, console, requests)
// printed to stdout for triage.
import { chromium } from "@playwright/test";

const URL = process.env.PRESENTER_URL || "http://10.77.8.134:8080/stage";

(async () => {
  const browser = await chromium.launch({
    channel: "chrome",
    args: ["--autoplay-policy=user-gesture-required"],
  });
  const ctx = await browser.newContext();
  const page = await ctx.newPage();

  const consoleMsgs = [];
  page.on("console", (m) => consoleMsgs.push(`[${m.type()}] ${m.text()}`));
  page.on("pageerror", (e) => consoleMsgs.push(`[pageerror] ${e.message}`));

  const requests = [];
  page.on("response", (r) => {
    const u = r.url();
    if (u.includes("/ndi/") || u.includes("/stage")) {
      requests.push(`${r.request().method()} ${u} -> ${r.status()}`);
    }
  });

  await page.goto(URL);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  const result = await page
    .locator('[data-role="ndi-video"]')
    .evaluate(async (el) => {
      const samples = [];
      for (let i = 0; i < 200; i++) {
        const snap = {
          t: i * 100,
          paused: el.paused,
          ct: el.currentTime,
          vw: el.videoWidth,
          rs: el.readyState,
          hasSrc: !!el.srcObject,
        };
        samples.push(snap);
        if (!el.paused && el.currentTime > 0.1) {
          return { ok: true, final: snap };
        }
        await new Promise((r) => setTimeout(r, 100));
      }
      const final = samples[samples.length - 1];
      return {
        ok: false,
        final,
        peakVw: Math.max(...samples.map((s) => s.vw)),
        peakRs: Math.max(...samples.map((s) => s.rs)),
        peakHasSrc: samples.some((s) => s.hasSrc),
      };
    });

  await browser.close();
  console.log("RESULT:", JSON.stringify(result, null, 2));
  console.log("CONSOLE:");
  consoleMsgs.slice(0, 30).forEach((m) => console.log("  " + m));
  console.log("REQUESTS:");
  requests.forEach((r) => console.log("  " + r));
  process.exit(result.ok ? 0 : 1);
})();
