/**
 * WASM Operator — slide "Group" field editing (#553).
 *
 * A worship-team volunteer preparing songs reported: after typing in one
 * slide's Group field, the page becomes unresponsive — clicking anywhere
 * else does nothing — until a full page reload. And the intended rule
 * ("set the group on slide N, every following slide inherits it until the
 * next slide with its own explicit group") was silently breaking: editing
 * an earlier slide's group could revert a later slide's group after the
 * fact.
 *
 * Root cause (confirmed by code + manual repro): the Group field's blur
 * handler was the only one of the four editable fields that (a) forced a
 * FULL tracked re-render of the entire (unkeyed) slide list, which fights
 * the async focus-restore workaround badly enough to loop forever — any
 * click elsewhere re-blurs the (just-refocused) Group input, re-triggering
 * another save+refetch+refocus cycle — and (b) raced a second `get_presentation`
 * refetch against a later edit with no staleness guard, so an out-of-order
 * response could silently revert a newer change.
 */

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

/** Create a presentation with exactly `count` slides via the HTTP API. */
async function createPresentationWithSlides(
  request: any,
  count: number,
  libraryName: string,
): Promise<{ presentationId: string; slideIds: string[] }> {
  const libResp = await request.post(new URL("/libraries", baseURL).toString(), {
    data: { name: libraryName },
  });
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Group field test song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: { presentation: { id: string; slides: Array<{ id: string }> } } =
    await presResp.json();
  const presentationId = presPayload.presentation.id;

  // create_presentation always seeds one slide; insert the rest at the end.
  let slideIds = presPayload.presentation.slides.map((s) => s.id);
  while (slideIds.length < count) {
    const insertResp = await request.post(
      new URL(`/presentations/${presentationId}/slides`, baseURL).toString(),
      { data: { position: null } },
    );
    expect(insertResp.ok()).toBeTruthy();
    const slides: Array<{ id: string }> = await insertResp.json();
    slideIds = slides.map((s) => s.id);
  }

  // Give every slide non-empty main text: the header group badge is
  // deliberately suppressed on a "true blank" slide (empty main AND no
  // explicit group, #275's empty-bookend-slide rule) so an inherited-group
  // badge needs real content to be visible at all.
  for (let i = 0; i < slideIds.length; i += 1) {
    const updateResp = await request.patch(
      new URL(`/presentations/${presentationId}/slides/${slideIds[i]}`, baseURL).toString(),
      { data: { main: `Slide ${i + 1}`, translation: "", stage: "" } },
    );
    expect(updateResp.ok()).toBeTruthy();
  }

  return { presentationId, slideIds };
}

/** Open a presentation in the operator UI via the global search box. */
async function openPresentation(page: import("@playwright/test").Page, name: string) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  const searchInput = page.locator('[data-role="global-search-query"]');
  await searchInput.fill(name);
  const result = page
    .locator('[data-role="search-result-item"][data-kind="presentation"]')
    .first();
  await expect(result).toBeVisible({ timeout: 15_000 });
  await result.click();

  await page.waitForFunction(
    () =>
      (document.querySelector('[data-role="slides"]')?.querySelectorAll(
        "[data-slide-id]",
      ).length ?? 0) > 0,
    { timeout: 15_000 },
  );

  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "edit",
    { timeout: 5_000 },
  );
}

test.describe("WASM Operator Group field (#553)", () => {
  test("editing a slide's group field does not freeze the page — you can immediately click another field", async ({
    page,
    request,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(`[pageerror] ${err.message}`));

    const libraryName = `E2E Group Focus ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 3, libraryName);
    await openPresentation(page, libraryName);

    const groupInput = page.locator(
      `[data-slide-id="${slideIds[0]}"] input[data-field="group"]`,
    );
    await groupInput.fill("Verse 1");
    await groupInput.blur();

    // Try to focus a DIFFERENT slide's main textarea, exactly what the
    // volunteer does next in real use. Before the fix, the Group field's
    // async save+refetch+refocus cycle steals focus back some time AFTER
    // the click (once the network round trip completes) — so a check made
    // IMMEDIATELY after the click can catch a false-positive "focused"
    // moment. Wait out the full async cycle first, THEN check where focus
    // actually ended up, and confirm real keystrokes land there (not a
    // Playwright `.fill()`, which re-focuses its target itself and would
    // mask the bug).
    const otherTextarea = page.locator(
      `[data-slide-id="${slideIds[1]}"] textarea[data-field="main"]`,
    );
    await otherTextarea.click();
    await page.waitForTimeout(1_000);

    const focusedSlide = await page.evaluate(
      () => document.activeElement?.closest("[data-slide-id]")?.getAttribute("data-slide-id") ?? null,
    );
    expect(
      focusedSlide,
      "focus must have moved to the slide the user clicked, not stayed stuck on the Group field",
    ).toBe(slideIds[1]);

    // The activeElement check above already proved focus genuinely moved;
    // `.fill()` here just confirms the field is now usable, replacing its
    // seed text rather than depending on cursor position.
    await otherTextarea.fill("typed after editing group");
    await expect(otherTextarea).toHaveValue("typed after editing group");

    expect(errors, `page errors: ${errors.join(" | ")}`).toEqual([]);
  });

  test("group value cascades forward across two edits and survives a reload", async ({
    page,
    request,
  }) => {
    const libraryName = `E2E Group Cascade ${Date.now()}`;
    const { presentationId, slideIds } = await createPresentationWithSlides(
      request,
      5,
      libraryName,
    );

    // Force the exact race that broke the cascade: delay slide[0]'s save
    // response so it resolves AFTER slide[3]'s, even though slide[0] was
    // edited first. Before the fix this let the (stale) slide[0] response
    // silently overwrite the presentation and revert slide[3]'s group.
    await page.route(
      `**/presentations/${presentationId}/slides/${slideIds[0]}`,
      async (route) => {
        if (route.request().method() === "PATCH") {
          await new Promise((r) => setTimeout(r, 800));
        }
        await route.continue();
      },
    );

    await openPresentation(page, libraryName);

    const badge = (slideId: string) =>
      page.locator(`[data-slide-id="${slideId}"] [data-role="slide-group"]`);

    const group0 = page.locator(
      `[data-slide-id="${slideIds[0]}"] input[data-field="group"]`,
    );
    // #556 F10: wait for the ACTUAL PATCH responses instead of a fixed
    // sleep that merely outlasts the deliberately-delayed 800ms request on
    // average — a banned arbitrary sleep, and a real flake risk on a
    // shared/loaded CI runner.
    const patch0Response = page.waitForResponse(
      (resp) =>
        resp.url().includes(`/presentations/${presentationId}/slides/${slideIds[0]}`) &&
        resp.request().method() === "PATCH",
    );
    await group0.fill("A");
    await group0.blur();
    // Don't wait for slide[0]'s (delayed) response — edit slide[3] right away.

    const group3 = page.locator(
      `[data-slide-id="${slideIds[3]}"] input[data-field="group"]`,
    );
    const patch3Response = page.waitForResponse(
      (resp) =>
        resp.url().includes(`/presentations/${presentationId}/slides/${slideIds[3]}`) &&
        resp.request().method() === "PATCH",
    );
    await group3.fill("B");
    await group3.blur();

    // Wait for BOTH PATCHes — including the deliberately delayed one — to
    // actually resolve.
    await Promise.all([patch0Response, patch3Response]);

    // #556 F2/F7: the cascade must be visible LIVE, before any reload. The
    // Group field's save uses `update_untracked` for `selected_pres` (to
    // avoid a full, unkeyed slide-list rebuild / focus loss — see
    // `slide_save.rs`), so an assertion that only ever checks AFTER a
    // reload would pass on the OLD buggy code too (a reload always
    // re-fetches fresh data) — it can't distinguish "cascades live" from
    // "cascades only after a full reload", which was exactly the #553
    // symptom this PR fixes.
    await expect(badge(slideIds[0])).toHaveText("A");
    await expect(badge(slideIds[1])).toHaveText("A");
    await expect(badge(slideIds[2])).toHaveText("A");
    await expect(badge(slideIds[3])).toHaveText("B");
    await expect(badge(slideIds[4])).toHaveText("B");

    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    // Re-open the same presentation after reload.
    await openPresentation(page, libraryName);

    await expect(badge(slideIds[0])).toHaveText("A");
    await expect(badge(slideIds[1])).toHaveText("A");
    await expect(badge(slideIds[2])).toHaveText("A");
    await expect(badge(slideIds[3])).toHaveText("B");
    await expect(badge(slideIds[4])).toHaveText("B");
  });
});
