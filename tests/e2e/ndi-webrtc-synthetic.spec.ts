import {
  test,
  expect,
  type Page,
  type APIRequestContext,
} from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// REQUIRED real-frame NDI→WebRTC tests (the regression guards for the
// "connected but black screen" bugs: #336, #372, #373).
//
// Unlike the capability-gated tests in ndi-webrtc.spec.ts, these tests do NOT
// skip — they assert that actual H264 frames decode in a real browser. They are
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

/** Discover the synthetic NDI source the lane published. The machine-name
 * prefix varies per host, so match on the "(PRESENTER-TEST)" suffix. NDI
 * discovery on a freshly-started server takes a few seconds, so poll (up to
 * ~30s) rather than querying once. */
async function discoverSyntheticSource(
  request: APIRequestContext,
): Promise<{ name: string } | undefined> {
  for (let i = 0; i < 30; i++) {
    const resp = await request.get(new URL("/ndi/sources", baseURL).toString());
    if (resp.ok()) {
      const list = await resp.json();
      if (Array.isArray(list)) {
        const synthetic = list.find((s: { name: string }) =>
          s.name.includes("(PRESENTER-TEST)"),
        );
        if (synthetic) return synthetic;
      }
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  return undefined;
}

/** Clean slate, then create + activate a video source for the synthetic NDI
 * name. Returns the created source row (with `.id`). */
async function createAndActivateSource(
  request: APIRequestContext,
  ndiName: string,
  label: string,
): Promise<{ id: string }> {
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label, ndiName } },
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
  return src;
}

/** Poll /healthz until the given source's pipeline reports `streaming`. */
async function waitForPipelineStreaming(
  request: APIRequestContext,
  sourceId: string,
): Promise<void> {
  for (let i = 0; i < 30; i++) {
    const resp = await request.get(new URL("/healthz", baseURL).toString());
    if (resp.ok()) {
      const body = await resp.json();
      const entry = (body.ndi_pipelines ?? []).find(
        (p: { source_id: string; state: string }) => p.source_id === sourceId,
      );
      if (entry && entry.state === "streaming") return;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`pipeline for source ${sourceId} never reached streaming`);
}

type InboundStats = {
  framesDecoded: number;
  bytesReceived: number;
  frameWidth: number;
  conn: string;
};

/** Connect `n` WHEP consumers nearly-simultaneously from the browser page,
 * each offering VIDEO + AUDIO recvonly (#336: the original "connected but
 * black" only dropped video when an audio m-line was also negotiated, so a
 * video-only offer would be a false pass), then poll getStats (up to ~25s)
 * until every consumer decodes a frame.
 *
 * Assert via getStats (framesDecoded / bytesReceived) not <video>.videoWidth:
 * headless Chrome decodes WebRTC media but does not reliably surface <video>
 * dimensions, so getStats is the precise measure. */
async function connectAndMeasure(
  page: Page,
  sourceId: string,
  n: number,
): Promise<{ error?: string; stats?: InboundStats[] }> {
  return page.evaluate(
    async ({ sourceId, n }) => {
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
          return { ok: false as const, reason: `WHEP POST ${resp.status}` };
        }
        await pc.setRemoteDescription({
          type: "answer",
          sdp: await resp.text(),
        });
        return { ok: true as const, pc };
      }

      const conns = await Promise.all(
        Array.from({ length: n }, () => connectOne()),
      );
      const bad = conns.find((c) => !c.ok);
      if (bad) {
        // Close the connections that DID succeed so partial failures don't
        // leave dangling peer connections (and server-side consumers) behind.
        for (const c of conns) {
          if (c.ok) (c as { pc: RTCPeerConnection }).pc.close();
        }
        return { error: (bad as { reason: string }).reason };
      }
      const pcs = conns.map((c) => (c as { pc: RTCPeerConnection }).pc);

      const read = async () =>
        Promise.all(
          pcs.map(async (pc) => {
            const out = {
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
    },
    { sourceId, n },
  );
}

/** Assert every consumer decoded frames AND the stream was downscaled.
 * The synthetic source is 2560×1440; the pipeline MUST downscale ≤1280 before
 * encoding or the browser cannot decode the high H264 level. */
function assertAllDecoded(stats: InboundStats[], scenario: string): void {
  stats.forEach((s, i) => {
    expect(
      s.framesDecoded,
      `[${scenario}] consumer ${i} must DECODE video frames (framesDecoded > 0); ` +
        `connected-but-zero-frames is the black-stage bug. Got: ${JSON.stringify(s)}`,
    ).toBeGreaterThan(0);
  });
  stats.forEach((s, i) => {
    expect(
      s.frameWidth,
      `[${scenario}] consumer ${i} decoded frame must be downscaled ≤1280 wide, got ${s.frameWidth}`,
    ).toBeLessThanOrEqual(1280);
  });
}

/** Collect console errors/warnings (browser-console-zero-errors rule). */
function collectConsoleErrors(page: Page): string[] {
  const consoleErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleErrors.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  return consoleErrors;
}

async function cleanupSource(
  request: APIRequestContext,
  sourceId: string,
): Promise<void> {
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${sourceId}`, baseURL).toString(),
  );
}

// ── Test 1: the #372 guard — a from-zero BURST of simultaneous consumers.
// The shipped bug delivered video to the FIRST consumer only — every
// additional one reached connectionState=connected but received ZERO RTP
// ("connected, black"), because a per-consumer webrtcbin added to the live
// pipeline had its rtpsession's latency unconfigured and dropped every
// outgoing packet. A single-consumer test passed while every real
// multi-display setup was black. We connect all three nearly-simultaneously
// (a from-zero burst, the way displays come up together) and require EVERY
// one to decode.
test("NDI video decodes real frames for MULTIPLE simultaneous consumers (synthetic source) @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSyntheticSource(request);
  // NOT a skip: on the e2e-ndi lane the synthetic sender MUST be running. If
  // it isn't, that is a real failure (broken lane), per test-strictness.
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();

  const src = await createAndActivateSource(
    request,
    synthetic!.name,
    "Synthetic-E2E-Burst",
  );

  const consoleErrors = collectConsoleErrors(page);
  await page.goto(new URL("/", baseURL).toString());

  // Connect immediately after activation — the from-zero burst.
  const results = await connectAndMeasure(page, src.id, 3);
  expect(
    results.error,
    `all WHEP POSTs must succeed — ${results.error}`,
  ).toBeFalsy();
  assertAllDecoded(results.stats!, "from-zero burst");

  expect(
    consoleErrors,
    `browser console must have zero errors/warnings, got: ${consoleErrors.join("; ")}`,
  ).toEqual([]);

  await cleanupSource(request, src.id);
});

type CompatStats = {
  framesDecoded: number;
  frameWidth: number;
  mimeType: string;
  conn: string;
};

/** Connect ONE WHEP consumer with a PLAIN offer (no codec games) to the
 * `?profile=compat` WHEP URL — exactly what the stage client does on weak
 * TVs whose MStar OMX H264 decoder is vendor-broken. The server must feed
 * that consumer the realtime-VP8 640×360@15 compat branch (every browser
 * offer carries VP8, so the plain offer needs no setCodecPreferences).
 * Waits ~8s for steady decode, then keeps polling getStats (up to ~25s
 * total) so a loaded runner doesn't flake the decode assertion. Returns the
 * inbound codec mimeType (inbound-rtp codecId → codec report) and
 * frameWidth so the test can assert the compat branch (not the 720p H264
 * default) actually fed the consumer. Releases the server-side consumer via
 * WHEP DELETE on the Location before returning. */
async function connectCompatAndMeasure(
  page: Page,
  origin: string,
  sourceId: string,
): Promise<{ error?: string; stats?: CompatStats }> {
  return page.evaluate(
    async ({ origin, sourceId }) => {
      const pc = new RTCPeerConnection();
      try {
        pc.addTransceiver("video", { direction: "recvonly" });
        pc.addTransceiver("audio", { direction: "recvonly" });

        // WHEP dance — same as the other tests in this file, except the
        // URL carries ?profile=compat (the profile selector; the answer
        // dictates the codec — realtime VP8 for compat).
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        await new Promise<void>((res) => {
          if (pc.iceGatheringState === "complete") return res();
          pc.addEventListener("icegatheringstatechange", () => {
            if (pc.iceGatheringState === "complete") res();
          });
          setTimeout(res, 4000);
        });
        const resp = await fetch(`/ndi/whep/${sourceId}?profile=compat`, {
          method: "POST",
          headers: { "Content-Type": "application/sdp" },
          body: pc.localDescription!.sdp,
        });
        if (!resp.ok) return { error: `WHEP POST ${resp.status}` };
        const location =
          resp.headers.get("Location") ?? resp.headers.get("location");
        await pc.setRemoteDescription({
          type: "answer",
          sdp: await resp.text(),
        });

        const read = async () => {
          const out = {
            framesDecoded: 0,
            frameWidth: 0,
            mimeType: "",
            conn: pc.connectionState,
          };
          const report = await pc.getStats();
          report.forEach((s) => {
            if (s.type === "inbound-rtp" && s.kind === "video") {
              out.framesDecoded = s.framesDecoded || 0;
              out.frameWidth = s.frameWidth || 0;
              const codec = s.codecId ? report.get(s.codecId) : undefined;
              out.mimeType = (codec && codec.mimeType) || "";
            }
          });
          return out;
        };
        // Sample after ~8s, then poll up to ~25s total for decode + width.
        await new Promise((r) => setTimeout(r, 8000));
        let stats = await read();
        for (let i = 0; i < 34; i++) {
          if (stats.framesDecoded > 0 && stats.frameWidth > 0) break;
          await new Promise((r) => setTimeout(r, 500));
          stats = await read();
        }

        // Release the server-side consumer (WHEP DELETE on the Location).
        if (location) {
          await fetch(new URL(location, origin).toString(), {
            method: "DELETE",
          }).catch(() => {});
        }
        return { stats };
      } catch (e) {
        return { error: String(e) };
      } finally {
        pc.close();
      }
    },
    { origin, sourceId },
  );
}

// ── Test 2: the #373 guard — STRAGGLER consumers joining an ALREADY-STREAMING
// pipeline. This is the dominant real-world scenario: a stage display loads
// /stage after the operator already activated the source, or a display's
// watchdog reconnects after a stall. The shipped bug: a webrtcbin added to a
// pipeline that had been PLAYING for a while never got its rtpsession latency
// configured ("Can't determine running time for this packet without knowing
// configured latency") and forwarded ZERO RTP — connected, but black — while
// the from-zero burst (test 1) passed, which is exactly how CI stayed green
// while every real stage display was black.
test("NDI video decodes for STRAGGLER consumers joining an already-streaming pipeline (synthetic source) @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSyntheticSource(request);
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();

  const src = await createAndActivateSource(
    request,
    synthetic!.name,
    "Synthetic-E2E-Straggler",
  );

  // Let the pipeline reach streaming and then run ALONE — zero consumers —
  // long enough that any later consumer is a genuine straggler (well past the
  // initial latency distribution of the PLAYING transition).
  await waitForPipelineStreaming(request, src.id);
  await new Promise((r) => setTimeout(r, 10_000));

  const consoleErrors = collectConsoleErrors(page);
  await page.goto(new URL("/", baseURL).toString());

  // NOW connect 3 simultaneous stragglers — every one must decode.
  const results = await connectAndMeasure(page, src.id, 3);
  expect(
    results.error,
    `all WHEP POSTs must succeed — ${results.error}`,
  ).toBeFalsy();
  assertAllDecoded(results.stats!, "straggler");

  expect(
    consoleErrors,
    `browser console must have zero errors/warnings, got: ${consoleErrors.join("; ")}`,
  ).toEqual([]);

  await cleanupSource(request, src.id);
});

// ── Test 3: the compat-profile guard (realtime-VP8 pivot in
// docs/superpowers/specs/2026-06-11-ndi-low-latency-design.md). The weak
// stage TVs' MStar OMX H264 decoder is vendor-broken (even the exactly-
// 640×480 H264 attempt died after ~5s, and 4:3 letterboxed the picture).
// VDO.Ninja's libwebrtc VP8 has played smoothly on the SAME TVs for years —
// the compat branch now mirrors it: realtime VP8 640×360@15, token-
// partitions=4 so the TV decodes on all 4 cores. The stage client's
// fallback re-POSTs its WHEP offer with ?profile=compat and the server must
// feed THAT consumer the VP8 branch. Two guards in one: (a) the decode
// guard — a compat-profile consumer gets decodable frames; (b) the profile
// guard — the decoded stream is EXACTLY 640 wide VP8, so the query really
// selected the compat branch (a 1280-wide H264 frame means the profile was
// ignored, the silent regression that would put the TVs back on the
// OMX-killing stream).
test("compat profile consumers get the realtime VP8 640x360 stream @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSyntheticSource(request);
  // NOT a skip: on the e2e-ndi lane the synthetic sender MUST be running. If
  // it isn't, that is a real failure (broken lane), per test-strictness.
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();

  const src = await createAndActivateSource(request, synthetic!.name, "compat");
  try {
    const consoleErrors = collectConsoleErrors(page);
    await page.goto(new URL("/", baseURL).toString());

    const result = await connectCompatAndMeasure(page, baseURL, src.id);
    expect(
      result.error,
      `WHEP connect must succeed — ${result.error}`,
    ).toBeFalsy();
    const s = result.stats!;
    console.log(`[e2e-evidence] compat-profile stats: ${JSON.stringify(s)}`);
    expect(
      s.framesDecoded,
      `compat-profile consumer must DECODE video frames (framesDecoded > 0); ` +
        `connected-but-zero-frames is the black-stage bug. Got: ${JSON.stringify(s)}`,
    ).toBeGreaterThan(0);
    // EXACTLY 640 — the 16:9 360p compat width. 1280 here means the
    // ?profile=compat query was ignored and the default branch leaked in.
    expect(
      s.frameWidth,
      `compat-profile frame must be EXACTLY 640 wide, got ${s.frameWidth}`,
    ).toBe(640);
    expect(
      s.mimeType,
      `compat profile must be VP8 (VDO.Ninja-style realtime stream — ` +
        `multithreaded SW decode on the TVs; their H264 OMX is vendor-broken), ` +
        `got: ${JSON.stringify(s)}`,
    ).toBe("video/VP8");

    expect(
      consoleErrors,
      `browser console must have zero errors/warnings, got: ${consoleErrors.join("; ")}`,
    ).toEqual([]);
  } finally {
    await cleanupSource(request, src.id);
  }
});

// ── Test 4: the deactivate→reactivate guard (prod TV white-screen incident,
// 2026-06). After the operator deactivates the active source and activates
// it again, the REAL stage page (WASM UI at /stage) must unmount the video,
// then REMOUNT it and resume decoding — without a page reload. The shipped
// bug: a TV that missed the ndi_source_activated live event (zombie WS /
// broadcast lag) showed a white stage with ZERO WHEP attempts until someone
// reloaded the page. This drives the same user-visible flow end-to-end
// through the stage UI's reactive chain (WS event → signals → <NdiVideo>).
test("stage remounts NDI video and resumes decoding after deactivate→reactivate (synthetic source) @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSyntheticSource(request);
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();

  const src = await createAndActivateSource(
    request,
    synthetic!.name,
    "Synthetic-E2E-Reactivate",
  );
  try {
    await waitForPipelineStreaming(request, src.id);

    // Real stage page on the ndi-fullscreen layout.
    const layoutResp = await request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "ndi-fullscreen" } },
    );
    expect(
      layoutResp.ok(),
      "switching stage layout to ndi-fullscreen must succeed",
    ).toBe(true);

    // Errors only: the deactivate phase legitimately emits watchdog/reconnect
    // console WARNINGS by design (same convention as ndi-webrtc-recovery).
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error") consoleErrors.push(msg.text());
    });

    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
      timeout: 10_000,
    });

    const video = page.locator('video[data-role="ndi-video"]');
    await expect(video).toBeVisible({ timeout: 15_000 });

    // Frames PRESENTED by the (current) video element — the same signal the
    // frame-based watchdog uses. Resets when the element is remounted.
    const framesPresented = () =>
      video.evaluate(
        (v: HTMLVideoElement) => v.getVideoPlaybackQuality().totalVideoFrames,
      );

    // Phase 1: initial playback presents frames.
    await expect
      .poll(framesPresented, {
        timeout: 25_000,
        message: "initial NDI playback never presented a frame",
      })
      .toBeGreaterThan(0);

    // Phase 2: deactivate server-side → the stage must unmount the video.
    await request.post(
      new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    );
    await expect(video).toHaveCount(0, { timeout: 10_000 });

    // Phase 3: reactivate → the stage must remount <NdiVideo> and decode
    // again WITHOUT a reload (white screen + zero WHEP attempts = the bug).
    const reactivate = await request.post(
      new URL(
        `/integrations/video-sources/${src.id}/activate`,
        baseURL,
      ).toString(),
      { data: {} },
    );
    expect(reactivate.ok(), "reactivating the source must succeed").toBe(true);

    await expect(video).toBeVisible({ timeout: 15_000 });
    await expect
      .poll(framesPresented, {
        timeout: 30_000,
        message:
          "stage never resumed presenting frames after deactivate→reactivate (bug-A regression)",
      })
      .toBeGreaterThan(0);

    expect(
      consoleErrors,
      `browser console must have zero errors, got: ${consoleErrors.join("; ")}`,
    ).toEqual([]);
  } finally {
    await cleanupSource(request, src.id);
  }
});
