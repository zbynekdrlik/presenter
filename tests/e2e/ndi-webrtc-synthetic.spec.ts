import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// REQUIRED real-frame NDI→WebRTC test (the regression guard for the "connected
// but black screen" bugs: #336, #372).
//
// Unlike the capability-gated tests in ndi-webrtc.spec.ts, this test does NOT
// skip — it asserts that actual H264 frames decode in a real browser. It is
// driven by the `e2e-ndi` self-hosted CI lane, which:
//   1. Starts the synthetic NDI sender (`ndi_test_sender`, publishes
//      "<host> (PRESENTER-TEST)") BEFORE Playwright runs, and
//   2. Runs ONLY this file (`--grep "@synthetic-ndi"`).
// The default ubuntu `e2e` job EXCLUDES it (`--grep-invert "@synthetic-ndi"`)
// because that runner has no NDI SDK / GPU encoder.
//
// Tags: @video-codec routes it to the real-Chrome (H.264) Playwright project;
// @synthetic-ndi selects it into the self-hosted lane.
// ─────────────────────────────────────────────────────────────────────────

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

test("NDI video decodes real frames for MULTIPLE simultaneous consumers (synthetic source) @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  // Discover the synthetic NDI source the lane published. The machine-name
  // prefix varies per host, so match on the "(PRESENTER-TEST)" suffix.
  // NDI discovery on a freshly-started server takes a few seconds, so poll
  // (up to ~30s) rather than querying once.
  let synthetic: { name: string } | undefined;
  for (let i = 0; i < 30; i++) {
    const resp = await request.get(new URL("/ndi/sources", baseURL).toString());
    if (resp.ok()) {
      const list = await resp.json();
      if (Array.isArray(list)) {
        synthetic = list.find((s: { name: string }) =>
          s.name.includes("(PRESENTER-TEST)"),
        );
        if (synthetic) break;
      }
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  // NOT a skip: on the e2e-ndi lane the synthetic sender MUST be running. If
  // it isn't, that is a real failure (broken lane), per test-strictness.
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();

  // Clean slate, then create + activate the synthetic source.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Synthetic-E2E", ndiName: synthetic!.name } },
  );
  expect(created.status(), "creating the video source must succeed").toBe(200);
  const src = await created.json();
  expect(
    (
      await request.post(
        new URL(
          `/integrations/video-sources/${src.id}/activate`,
          baseURL,
        ).toString(),
        { data: {} },
      )
    ).status(),
  ).toBe(200);

  // ── The core regression guard: real H264 frames must DECODE over WebRTC in a
  // real browser, for MULTIPLE consumers that connect at once — the actual
  // stage-display scenario (every TV/laptop mounts <NdiVideo> when the source
  // is triggered).
  //
  // This is THREE consumers, not one, on purpose (#372): the shipped bug
  // delivered video to the FIRST consumer only — every additional one reached
  // connectionState=connected but received ZERO RTP ("connected, black"),
  // because a per-consumer webrtcbin added to the live pipeline had its
  // rtpsession's latency unconfigured and dropped every outgoing packet. A
  // single-consumer test passed while every real multi-display setup was black.
  // We connect all three nearly-simultaneously (a from-zero burst, the way
  // displays come up together) and require EVERY one to decode.
  //
  // Each offers VIDEO + AUDIO (#336): the original "connected but black" only
  // dropped video when an audio m-line was also negotiated, so a video-only
  // offer was a false pass.
  //
  // Assert via getStats (framesDecoded / bytesReceived) not <video>.videoWidth:
  // headless Chrome decodes WebRTC media but does not reliably surface <video>
  // dimensions, so getStats is the precise measure.
  // Collect console errors/warnings (browser-console-zero-errors rule): a
  // WASM panic or page error must fail the test, not slip by silently.
  const consoleErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleErrors.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await page.goto(new URL("/", baseURL).toString());
  const results = await page.evaluate(async (sourceId) => {
    async function connectOne() {
      const pc = new RTCPeerConnection();
      pc.addTransceiver("video", { direction: "recvonly" });
      pc.addTransceiver("audio", { direction: "recvonly" });
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      await new Promise<void>((res) => {
        if (pc.iceGatheringState === "complete") return res();
        pc.addEventListener("icegatheringstatechange", () => {
          if (pc.iceGatheringState === "complete") res();
        });
        setTimeout(res, 4000);
      });
      const resp = await fetch(`/ndi/whep/${sourceId}`, {
        method: "POST",
        headers: { "Content-Type": "application/sdp" },
        body: pc.localDescription!.sdp,
      });
      if (!resp.ok) {
        pc.close();
        return { ok: false, reason: `WHEP POST ${resp.status}` };
      }
      await pc.setRemoteDescription({ type: "answer", sdp: await resp.text() });
      return { ok: true, pc };
    }

    // Connect all three at once (from-zero burst).
    const conns = await Promise.all([connectOne(), connectOne(), connectOne()]);
    const bad = conns.find((c) => !c.ok);
    if (bad) return { error: (bad as { reason: string }).reason };

    const pcs = conns.map((c) => (c as { pc: RTCPeerConnection }).pc);

    // Poll up to ~25s for every consumer to decode at least one frame.
    type Inb = {
      framesDecoded: number;
      bytesReceived: number;
      frameWidth: number;
      conn: string;
    };
    const read = async (): Promise<Inb[]> =>
      Promise.all(
        pcs.map(async (pc) => {
          const out: Inb = {
            framesDecoded: 0,
            bytesReceived: 0,
            frameWidth: 0,
            conn: pc.connectionState,
          };
          (await pc.getStats()).forEach((s) => {
            if (s.type === "inbound-rtp" && s.kind === "video") {
              out.framesDecoded = s.framesDecoded || 0;
              out.bytesReceived = s.bytesReceived || 0;
              out.frameWidth = s.frameWidth || 0;
            }
          });
          return out;
        }),
      );
    let stats = await read();
    for (let i = 0; i < 50; i++) {
      await new Promise((r) => setTimeout(r, 500));
      stats = await read();
      if (stats.every((s) => s.framesDecoded > 0)) break;
    }
    pcs.forEach((pc) => pc.close());
    return { stats };
  }, src.id);

  expect(
    results.error,
    `all WHEP POSTs must succeed — ${results.error}`,
  ).toBeFalsy();
  const stats = results.stats!;
  // EVERY consumer must decode frames — the #372 multi-consumer guard.
  stats.forEach((s, i) => {
    expect(
      s.framesDecoded,
      `consumer ${i} must DECODE video frames (framesDecoded > 0); ` +
        `connected-but-zero-frames is the black-stage bug. Got: ${JSON.stringify(s)}`,
    ).toBeGreaterThan(0);
  });
  // The synthetic source is 2560×1440; the pipeline MUST downscale ≤1280 before
  // encoding or the browser cannot decode the high H264 level (the bug above
  // re-triggers). Assert the decoded frame is actually downscaled.
  stats.forEach((s, i) => {
    expect(
      s.frameWidth,
      `consumer ${i} decoded frame must be downscaled ≤1280 wide, got ${s.frameWidth}`,
    ).toBeLessThanOrEqual(1280);
  });

  // The browser console must be clean throughout (no WASM panic / page error).
  expect(
    consoleErrors,
    `browser console must have zero errors/warnings, got: ${consoleErrors.join("; ")}`,
  ).toEqual([]);

  // Cleanup.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});
