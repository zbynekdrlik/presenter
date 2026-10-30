/**
 * #850 B1b — touch-safe Worship slides (spec §6.5, S5). A finger that starts
 * a scroll on a slide card must never send it (the press used to trigger at
 * once); a tap sends exactly once; a mouse still sends on press (no added
 * latency for the FOH PC). The slide toolbar is 44 px under a finger.
 */
import {
  test,
  expect,
  type Browser,
  type BrowserContextOptions,
  type Page,
  type Request,
} from "@playwright/test";
import {
  assertMinTapTarget,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  touchDrag,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";
let presentationId = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
  const lib = await (
    await fetch(new URL("/libraries", baseURL), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: `TouchSafe${Date.now()}` }),
    })
  ).json();
  const slides = Array.from({ length: 12 }, (_, i) => ({ main: `Sloha ${i + 1}\nDruhý riadok` }));
  const created = await (
    await fetch(new URL(`/libraries/${lib.id}/presentations`, baseURL), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Touch Safe Song", slides }),
    })
  ).json();
  presentationId = created.presentation.id;
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function openSong(browser: Browser, profile: BrowserContextOptions) {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const sends: Array<{ currentSlideId: string }> = [];
  page.on("request", (req: Request) => {
    if (req.method() === "POST" && new URL(req.url()).pathname === "/stage/state") {
      sends.push(req.postDataJSON());
    }
  });
  await page.goto("/ui/operator");
  await page.evaluate((id) => {
    sessionStorage.setItem("presenter:currentPresentationId", JSON.stringify(id));
    sessionStorage.setItem("presenter:view", JSON.stringify("worship"));
    sessionStorage.setItem("presenter:mode", JSON.stringify("live"));
  }, presentationId);
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector(".operator__slides .operator__slide-card", { state: "visible" });
  return { context, page, errors, sends };
}

const cards = (page: Page) => page.locator(".operator__slides [data-slide-id]");

test("phone: a finger that scrolls from a slide sends nothing; a tap sends exactly that slide", async ({
  browser,
}) => {
  const { context, page, errors, sends } = await openSong(browser, PHONE_390);
  try {
    await cards(page).nth(0).scrollIntoViewIfNeeded();
    const first = await cards(page).nth(0).boundingBox();
    expect(first).not.toBeNull();
    const x = first!.x + first!.width / 2;
    const y = first!.y + first!.height / 2;
    await touchDrag(page, { x, y }, { x, y: y - 120 });

    // Ordering proves the drag sent nothing: the first send is the tap's.
    const tapped = cards(page).nth(1);
    await tapped.scrollIntoViewIfNeeded();
    await tapped.tap();
    await expect.poll(() => sends.length).toBe(1);
    expect(sends[0].currentSlideId).toBe(await tapped.getAttribute("data-slide-id"));
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: a mouse still sends on press, and its click sends nothing more", async ({ browser }) => {
  const { context, page, errors, sends } = await openSong(browser, { viewport: { width: 1280, height: 800 } });
  try {
    await cards(page).nth(2).scrollIntoViewIfNeeded();
    const box = await cards(page).nth(2).boundingBox();
    expect(box).not.toBeNull();
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
    const onPress = page.waitForRequest(
      (req) => req.method() === "POST" && new URL(req.url()).pathname === "/stage/state",
    );
    await page.mouse.down();
    await onPress; // before the button is released
    await page.mouse.up();

    // The next send is the next card's: the click after the press added none.
    await cards(page).nth(3).click();
    await expect.poll(() => sends.length).toBe(2);
    expect(sends[0].currentSlideId).toBe(await cards(page).nth(2).getAttribute("data-slide-id"));
    expect(sends[1].currentSlideId).toBe(await cards(page).nth(3).getAttribute("data-slide-id"));
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("phone: the slide toolbar controls are 44 px", async ({ browser }) => {
  const { context, page, errors } = await openSong(browser, PHONE_390);
  try {
    await assertMinTapTarget(
      page.locator(
        '[data-role="slide-columns-decrease"]:visible, [data-role="slide-columns-increase"]:visible, [data-role="add-slide"]:visible',
      ),
      44,
    );
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
