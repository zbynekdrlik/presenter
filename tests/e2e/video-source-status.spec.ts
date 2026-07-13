import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #546 — the settings page must SAY why a mapped NDI source is not showing video.
//
// At PP an operator mapped `cgpp → RESOLUME-PP (cg-obs)`, activated it, and the stage
// stayed blank with no explanation anywhere in the UI. The sending machine simply was
// not broadcasting that name. Hours went into the (genuinely broken) encoder chain
// before the log revealed it. The server knew all along; nothing told the human.
//
// This spec runs on the DEFAULT lane, whose HOST may or may not have libndi: the
// GitHub runners do not, dev2 — where the same suite is run before a merge — does. So
// it asserts the contract that must hold EITHER WAY and takes its branch from what the
// server reports about itself:
//
//   * blind server (no SDK) → `unknown` / "NDI unavailable". It must NEVER say "not
//     found on the network" about a network it cannot see; that sends the operator off
//     to check a sending machine that is perfectly fine.
//   * seeing server        → a name nobody broadcasts reads `not-found`, with the hint
//     and the list of what IS on the air. That is the PP reproduction.
//
// Hard-coding the SDK-less branch made this spec pass on CI and fail on dev2 — a guard
// that is green on only one host is not a guard. The full synthetic reproduction (a
// mapped ghost name while a REAL NDI sender is on the air) is in
// `ndi-source-status.spec.ts`, on the self-hosted lane.

test.describe.configure({ timeout: 180_000 });

/** A name nothing on any of our networks broadcasts. */
const GHOST = "GHOST-SOURCE (nope)";

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

async function createSource(request: any, label: string, ndiName: string) {
  const res = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label, ndiName } },
  );
  expect(res.status()).toBe(200);
  return await res.json();
}

async function readStatus(request: any) {
  const res = await request.get(
    new URL("/integrations/video-sources/status", baseURL).toString(),
  );
  expect(res.status()).toBe(200);
  return await res.json();
}

test("status endpoint joins the rows, the network and the pipelines", async ({
  request,
}) => {
  const source = await createSource(request, "cgpp", GHOST);

  const body = await readStatus(request);

  const entry = body.sources.find((s: any) => s.id === source.id);
  expect(entry, "the created source must appear in the status snapshot").toBeTruthy();
  expect(entry.ndiName).toBe(GHOST);

  if (body.ndiAvailable) {
    // The PP case: the mapped name is simply not being broadcast.
    expect(body.discovered).not.toContain(GHOST);
    expect(entry.state).toBe("not-found");
  } else {
    // A blind server says so — it never accuses a sender it cannot see.
    expect(body.discovered).toEqual([]);
    expect(entry.state).toBe("unknown");
  }
});

test("the settings card shows the source's status badge", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  const source = await createSource(request, "cam-status", GHOST);
  const ndiAvailable: boolean = (await readStatus(request)).ndiAvailable;

  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 60_000 });

  const badge = page.locator(
    `[data-role="video-source-status"][data-source-id="${source.id}"]`,
  );
  const hint = page.locator(
    `[data-role="video-source-hint"][data-source-id="${source.id}"]`,
  );

  if (ndiAvailable) {
    await expect(badge).toHaveText("Not found on the network", { timeout: 30_000 });
    await expect(badge).toHaveAttribute("data-state", "not-found");
    // The sentence that would have ended the PP outage in a minute.
    await expect(hint).toContainText("not on the network", { timeout: 30_000 });
    // And what IS on the air, right there, to compare the mapped name against.
    await expect(page.locator('[data-role="ndi-discovered"]')).toHaveCount(1);
  } else {
    await expect(badge).toHaveText("NDI unavailable", { timeout: 30_000 });
    await expect(badge).toHaveAttribute("data-state", "unknown");
    // Nothing can be discovered without the SDK, so the "on the network now" line must
    // not be there claiming an empty network.
    await expect(page.locator('[data-role="ndi-discovered"]')).toHaveCount(0);
  }

  expect(errors, `console errors: ${errors.join(" | ")}`).toEqual([]);
});
