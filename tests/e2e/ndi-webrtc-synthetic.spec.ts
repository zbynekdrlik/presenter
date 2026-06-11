import { test, expect, type Page, type APIRequestContext } from "@playwright/test";
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
function assertAllDecoded(
  stats: InboundStats[],
  scenario: string,
): void {
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

type Vp8Stats = {
  framesDecoded: number;
  frameWidth: number;
  mimeType: string;
  conn: string;
};

/** Connect ONE WHEP consumer that strips H264 from its video offer via
 * setCodecPreferences (keeping only VP8 + rtx) — exactly what the stage
 * client does on Vestel TVs whose vendor OMX H264 decoder is broken. The
 * server must answer with (and actually SEND) VP8 for that consumer. Waits
 * ~8s for steady decode, then keeps polling getStats (up to ~25s total) so
 * a loaded runner doesn't flake the decode assertion. Returns the inbound
 * codec mimeType (inbound-rtp codecId → codec report) so the test can
 * assert the negotiation really landed on VP8, not H264. Releases the
 * server-side consumer via WHEP DELETE on the Location before returning. */
async function connectVp8OnlyAndMeasure(
  page: Page,
  origin: string,
  sourceId: string,
): Promise<{ error?: string; stats?: Vp8Stats }> {
  return page.evaluate(
    async ({ origin, sourceId }) => {
      const pc = new RTCPeerConnection();
      try {
        pc.addTransceiver("video", { direction: "recvonly" });
        // Strip H264 from the offer: keep only VP8 (+ rtx retransmission).
        const tr =
          pc
            .getTransceivers()
            .find((t) => t.receiver?.track?.kind === "video") ??
          pc.getTransceivers()[0];
        const caps = RTCRtpReceiver.getCapabilities("video");
        tr.setCodecPreferences(
          caps!.codecs.filter((c) => /\/(VP8|rtx)$/i.test(c.mimeType)),
        );
        pc.addTransceiver("audio", { direction: "recvonly" });

        // WHEP dance — same as the other tests in this file.
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

// ── Test 3: the Vestel-OMX fallback guard (spec addendum 2 in
// docs/superpowers/specs/2026-06-11-ndi-low-latency-design.md). Some stage
// TVs (Vestel) ship a vendor OMX H264 decoder that silently fails, so the
// stage client strips H264 from its WHEP offer via setCodecPreferences and
// the server must fall back to encoding VP8 for THAT consumer. Two guards
// in one: (a) the decode guard — an H264-less offer still gets decodable,
// downscaled frames; (b) the negotiation guard — the inbound codec really
// is VP8, so the server didn't sneak H264 past the preference filter.
test("NDI stream decodes via VP8 for consumers that exclude H264 (Vestel OMX fallback path) @video-codec @synthetic-ndi", async ({
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

  const src = await createAndActivateSource(request, synthetic!.name, "vp8");
  try {
    const consoleErrors = collectConsoleErrors(page);
    await page.goto(new URL("/", baseURL).toString());

    const result = await connectVp8OnlyAndMeasure(page, baseURL, src.id);
    expect(
      result.error,
      `WHEP connect must succeed — ${result.error}`,
    ).toBeFalsy();
    const s = result.stats!;
    expect(
      s.framesDecoded,
      `VP8-only consumer must DECODE video frames (framesDecoded > 0); ` +
        `connected-but-zero-frames is the black-stage bug. Got: ${JSON.stringify(s)}`,
    ).toBeGreaterThan(0);
    expect(
      s.frameWidth,
      `decoded frame must have a real width (>0), got ${JSON.stringify(s)}`,
    ).toBeGreaterThan(0);
    expect(
      s.frameWidth,
      `decoded frame must be downscaled ≤1280 wide, got ${s.frameWidth}`,
    ).toBeLessThanOrEqual(1280);
    expect(
      s.mimeType,
      `server must actually serve VP8 to an H264-less offer, got: ${JSON.stringify(s)}`,
    ).toBe("video/VP8");

    expect(
      consoleErrors,
      `browser console must have zero errors/warnings, got: ${consoleErrors.join("; ")}`,
    ).toEqual([]);
  } finally {
    await cleanupSource(request, src.id);
  }
});
