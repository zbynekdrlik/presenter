// Spec #336 / Task 8: assert two concurrent WHEP consumers on the same
// NDI source share a single encoder and spawn two webrtcbins.
//
// Runs on dev2 (which IS the self-hosted CI runner). Requires an NDI
// broadcaster on the LAN; mirrors the skip pattern from
// ndi-webrtc-recovery.spec.ts for the case where no broadcaster is present.
//
// Tagged @video-codec so playwright.config.ts routes it through real Chrome
// (channel: "chrome" + --autoplay-policy=user-gesture-required) — the
// default Chromium build has no H264 codec and would not decode NDI video.

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  attachConsoleErrorCollector,
  waitForVideoReady,
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

// ─────────────────────────────────────────────────────────────────────────
// The load-bearing end-to-end proof for #336:
//
// Two concurrent browser contexts connect to the same NDI source via WHEP.
// The shared-encoder architecture must route both consumers through the
// SHARED per-profile encoders rather than spawning a second encoder per
// consumer. The encoder pipeline holds EXACTLY TWO encoders BY DESIGN —
// one per PROFILE (720p default + 640×480 compat) — regardless of how many
// consumers attach. The /ndi/snapshot/:source_id diagnostic route exposes
// this invariant so the test can assert it.
//
// Fanout invariant: encoderCount=2 (per profile, NOT per consumer),
// consumerCount=2.
// ─────────────────────────────────────────────────────────────────────────
test(
  "#336 shared-encoder fanout: two browser tabs → one encoder + two webrtcbins @video-codec",
  async ({ browser, request }) => {
    // Capability gate: skip when no NDI source is discoverable (CI without
    // a real LAN broadcaster). Mirrors ndi-webrtc-recovery.spec.ts exactly.
    const sourcesResp = await request.get(
      new URL("/ndi/sources", baseURL).toString(),
    );
    if (!sourcesResp.ok()) {
      test.skip(
        true,
        "NDI SDK not available on this runner (/ndi/sources not OK)",
      );
      return;
    }
    const discovered = await sourcesResp.json();
    if (!Array.isArray(discovered) || discovered.length === 0) {
      test.skip(true, "no NDI sources discovered on this runner");
      return;
    }

    // Create + activate a video_source row for the first discovered broadcaster.
    const ndiName = (discovered[0] as { name: string }).name;
    const created = await request.post(
      new URL("/integrations/video-sources", baseURL).toString(),
      { data: { label: "fanout-test", ndiName } },
    );
    expect(created.status()).toBeLessThan(500);
    const src = (await created.json()) as { id: string };

    const activate = await request.post(
      new URL(
        `/integrations/video-sources/${src.id}/activate`,
        baseURL,
      ).toString(),
      { data: {} },
    );
    expect(activate.status()).toBe(200);

    // Set the stage layout to ndi-fullscreen so /stage renders NdiVideo.
    await request.post(new URL("/stage/layout", baseURL).toString(), {
      data: { code: "ndi-fullscreen" },
    });

    // Open the stage in two concurrent browser contexts.
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    const errs1: string[] = [];
    const errs2: string[] = [];
    attachConsoleErrorCollector(page1, errs1);
    attachConsoleErrorCollector(page2, errs2);

    const stageUrl = new URL("/stage", baseURL).toString();
    await Promise.all([page1.goto(stageUrl), page2.goto(stageUrl)]);

    // Wait for WASM mount on both pages.
    await Promise.all([
      page1.waitForSelector('body[data-wasm-ready="true"]', {
        timeout: 30_000,
      }),
      page2.waitForSelector('body[data-wasm-ready="true"]', {
        timeout: 30_000,
      }),
    ]);
    await Promise.all([
      page1.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
        timeout: 10_000,
      }),
      page2.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
        timeout: 10_000,
      }),
    ]);

    // Both tabs must reach videoWidth > 0 on their ndi-video element.
    await Promise.all([
      waitForVideoReady(page1, '[data-role="ndi-video"]'),
      waitForVideoReady(page2, '[data-role="ndi-video"]'),
    ]);

    // Fanout invariant: one shared encoder, two consumers (webrtcbins).
    const snapResp = await request.get(
      new URL(`/ndi/snapshot/${encodeURIComponent(src.id)}`, baseURL).toString(),
    );
    expect(snapResp.ok(), `snapshot route returned ${snapResp.status()}`).toBe(
      true,
    );
    const snap = (await snapResp.json()) as {
      encoderCount: number;
      consumerCount: number;
      sessions: unknown[];
    };
    expect(
      snap.encoderCount,
      "shared-encoder invariant: exactly TWO encoders (one per profile: " +
        "720p default + 640x480 compat) for multiple consumers — never " +
        "one per consumer",
    ).toBe(2);
    expect(
      snap.consumerCount,
      "fanout invariant: two consumers attached to the shared pipeline",
    ).toBe(2);
    expect(
      snap.sessions,
      "sessions array must have one entry per consumer",
    ).toHaveLength(2);

    // Both browser consoles must be error-free.
    expect(errs1, "page1 console must be error-free").toEqual([]);
    expect(errs2, "page2 console must be error-free").toEqual([]);

    // Cleanup.
    await ctx1.close();
    await ctx2.close();
    await request.post(
      new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    );
    await request.delete(
      new URL(
        `/integrations/video-sources/${src.id}`,
        baseURL,
      ).toString(),
    );
  },
);
