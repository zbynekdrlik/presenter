// Test N concurrent WHEP consumers — reproduces user's multi-browser load.
import { chromium } from "@playwright/test";

const URL = process.env.PRESENTER_URL || "http://10.77.8.134:8080/stage";
const N = parseInt(process.env.N || "3", 10);

async function oneConsumer(idx) {
  const browser = await chromium.launch({
    channel: "chrome",
    args: ["--autoplay-policy=user-gesture-required"],
  });
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  const consoleMsgs = [];
  page.on("console", (m) =>
    consoleMsgs.push(`[${m.type()}] ${m.text()}`),
  );
  try {
    await page.goto(URL);
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    const result = await page
      .locator('[data-role="ndi-video"]')
      .evaluate(async (el) => {
        for (let i = 0; i < 200; i++) {
          if (!el.paused && el.currentTime > 0.1 && el.videoWidth > 0) {
            return {
              ok: true,
              paused: el.paused,
              ct: el.currentTime,
              vw: el.videoWidth,
              muted: el.muted,
              autoplayAttr: el.hasAttribute("autoplay"),
            };
          }
          await new Promise((r) => setTimeout(r, 100));
        }
        return {
          ok: false,
          paused: el.paused,
          ct: el.currentTime,
          vw: el.videoWidth,
          hasSrc: !!el.srcObject,
          muted: el.muted,
          autoplayAttr: el.hasAttribute("autoplay"),
        };
      });
    return { idx, console: consoleMsgs.filter((m) => !m.includes("404")), ...result };
  } finally {
    await browser.close();
  }
}

(async () => {
  console.log(`Launching ${N} concurrent consumers against ${URL}`);
  const results = await Promise.all(
    Array.from({ length: N }, (_, i) => oneConsumer(i + 1)),
  );
  for (const r of results) {
    const { idx, console: msgs, ...rest } = r;
    console.log(`Consumer ${idx}: ${JSON.stringify(rest)}`);
    if (msgs.length) {
      msgs.slice(0, 5).forEach((m) => console.log(`  ${m}`));
    }
  }
  const failed = results.filter((r) => !r.ok);
  if (failed.length > 0) {
    console.log(`FAILED: ${failed.length}/${N} consumers did not play`);
    process.exit(1);
  } else {
    console.log(`OK: all ${N} consumers playing`);
    process.exit(0);
  }
})();
