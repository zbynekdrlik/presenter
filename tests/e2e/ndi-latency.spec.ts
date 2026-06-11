import { test, expect, type Page, type APIRequestContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// Glass-to-glass latency guard for the NDI→WebRTC path.
//
// The synthetic sender (`ndi_test_sender`) bakes `Date.now() % 2^24` into a
// pixel clock strip on every frame (`presenter_ndi::test_strip`): 26 blocks
// of 48×48 px at (48,48) @2560×1440 — block 0 white, block 1 black
// (threshold calibration), blocks 2..=25 the 24-bit big-endian timestamp.
// After the server's downscale to 1280×720 the strip lands at row centre
// y=36, block i centre x = 24 + i*24 + 12 (24-px blocks). This test decodes
// the strip from the rendered video via canvas per displayed frame
// (requestVideoFrameCallback). Sender and browser run on the SAME machine
// (self-hosted e2e-ndi lane) → same clock → per-frame glass-to-glass latency
// = (Date.now() % 2^24 − decoded), mod-wrapped.
//
// Like ndi-webrtc-synthetic.spec.ts this file is driven by the `e2e-ndi`
// self-hosted CI lane (`--grep "@synthetic-ndi"`); the GitHub-hosted `e2e`
// job excludes it (`--grep-invert "@synthetic-ndi"`) — no NDI SDK there.
// @video-codec routes it to the real-Chrome (H.264) Playwright project.
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

/** Poll /healthz until the given source's pipeline reports `streaming` —
 * latency is a steady-state property, so don't measure through the pipeline
 * ramp-up. */
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

type LatencyResult = {
  n: number;
  badFrames: number;
  medianMs: number;
  p95Ms: number;
  freezeDurS: number;
};

/** Connect ONE WHEP consumer (video+audio recvonly, same dance as
 * ndi-webrtc-synthetic.spec.ts), render the stream into a hidden <video>,
 * and for `seconds` decode the baked clock strip from every displayed frame
 * via canvas + requestVideoFrameCallback. Returns sorted-sample stats. */
async function measureGlassToGlass(
  page: Page,
  origin: string,
  sourceId: string,
  seconds: number,
): Promise<{ error: string } | LatencyResult> {
  return page.evaluate(
    async ({ origin, sourceId, seconds }) => {
      const pc = new RTCPeerConnection();
      try {
        pc.addTransceiver("video", { direction: "recvonly" });
        pc.addTransceiver("audio", { direction: "recvonly" });
        const video = document.createElement("video");
        video.muted = true;
        video.autoplay = true;
        video.playsInline = true;
        document.body.appendChild(video);
        const canvas = document.createElement("canvas");
        canvas.width = 1280;
        canvas.height = 720;
        const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
        const samples: number[] = [];
        let badFrames = 0;
        let active = true;
        // Decode the 1280×720-downscaled clock strip: 24-px blocks, row
        // centre y=36, block i centre x = 24 + i*24 + 12. Average a 3×3
        // patch of luma around each centre to ride out encode ringing.
        function decodeStrip(): number | null {
          ctx.drawImage(video, 0, 0, 1280, 720);
          const y = 36;
          const luma = (i: number): number => {
            const x = 24 + i * 24 + 12;
            const d = ctx.getImageData(x - 1, y - 1, 3, 3).data;
            let s = 0;
            for (let p = 0; p < d.length; p += 4)
              s += 0.299 * d[p] + 0.587 * d[p + 1] + 0.114 * d[p + 2];
            return s / (d.length / 4);
          };
          const white = luma(0);
          const black = luma(1);
          if (white - black < 60) return null;
          const thr = (white + black) / 2;
          let val = 0;
          for (let bit = 0; bit < 24; bit++)
            val = (val << 1) | (luma(2 + bit) > thr ? 1 : 0);
          return val >>> 0;
        }
        function onFrame() {
          const embedded = decodeStrip();
          if (embedded === null) {
            badFrames++;
          } else {
            const now = Date.now() % (1 << 24);
            let d = now - embedded;
            if (d < -(1 << 23)) d += 1 << 24;
            if (d > 1 << 23) d -= 1 << 24;
            samples.push(d);
          }
          if (active) (video as any).requestVideoFrameCallback(onFrame);
        }
        pc.ontrack = (ev) => {
          if (ev.track.kind === "video") {
            video.srcObject = new MediaStream([ev.track]);
            video.play().catch(() => {});
            (video as any).requestVideoFrameCallback(onFrame);
          }
        };

        // WHEP dance — same as ndi-webrtc-synthetic.spec.ts.
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

        await new Promise((r) => setTimeout(r, seconds * 1000));
        active = false;

        let freezeDurS = 0;
        (await pc.getStats()).forEach((s) => {
          if (s.type === "inbound-rtp" && s.kind === "video") {
            freezeDurS = (s as any).totalFreezesDuration || 0;
          }
        });

        // Release the server-side consumer (WHEP DELETE on the Location).
        if (location) {
          await fetch(new URL(location, origin).toString(), {
            method: "DELETE",
          }).catch(() => {});
        }

        samples.sort((a, b) => a - b);
        const n = samples.length;
        const medianMs = n > 0 ? samples[Math.floor(n / 2)] : Number.NaN;
        const p95Ms =
          n > 0 ? samples[Math.min(n - 1, Math.floor(n * 0.95))] : Number.NaN;
        return { n, badFrames, medianMs, p95Ms, freezeDurS };
      } catch (e) {
        return { error: String(e) };
      } finally {
        pc.close();
      }
    },
    { origin, sourceId, seconds },
  );
}

// Glass-to-glass latency guard: the synthetic sender bakes Date.now()%2^24
// into a pixel strip (presenter_ndi::test_strip); this test decodes it from
// the rendered video via canvas per displayed frame. Sender and browser run
// on the SAME machine (self-hosted lane) -> same clock, true g2g latency.
//
// Bounds are deliberately generous for the shared runner (quiet-machine
// reality after the low-latency package is ~120-160ms median): a regression
// back to seconds-level latency or to growing-buffer behavior fails hard.
test("NDI glass-to-glass latency stays low for the synthetic source @video-codec @synthetic-ndi", async ({
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

  const src = await createAndActivateSource(request, synthetic!.name, "lat");
  try {
    // Latency is a steady-state property — measure after the pipeline
    // reports streaming, not through its ramp-up.
    await waitForPipelineStreaming(request, src.id);

    const consoleMessages = collectConsoleErrors(page);
    // /healthz is origin-correct for the relative WHEP fetches but doesn't
    // load the WASM UI — the measurement page stays inert.
    await page.goto(new URL("/healthz", baseURL).toString());

    const result = await measureGlassToGlass(page, baseURL, src.id, 20);
    expect(
      (result as { error?: string }).error,
      "WHEP connect must succeed",
    ).toBeUndefined();
    const r = result as LatencyResult;
    console.log(
      `[g2g] n=${r.n} badFrames=${r.badFrames} medianMs=${r.medianMs} ` +
        `p95Ms=${r.p95Ms} freezeDurS=${r.freezeDurS}`,
    );
    expect(
      r.n,
      `decoded strip samples (badFrames=${r.badFrames})`,
    ).toBeGreaterThan(300);
    expect(r.medianMs, "median glass-to-glass latency").toBeLessThanOrEqual(
      350,
    );
    expect(r.p95Ms, "p95 glass-to-glass latency").toBeLessThanOrEqual(600);
    expect(r.freezeDurS, "total freeze duration").toBeLessThan(1.0);
    expect(consoleMessages).toEqual([]);
  } finally {
    await cleanupSource(request, src.id);
  }
});
