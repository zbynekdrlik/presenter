import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// REQUIRED real-frame NDI→WebRTC test (the regression guard for the #336
// "connected but black screen" bug).
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

test("NDI video decodes real frames end-to-end (synthetic source) @video-codec @synthetic-ndi", async ({
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
  // Require a real success — a 4xx would pass `<500` but yield src.id===undefined
  // and a confusing downstream 404 instead of failing at the real cause.
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

  // ── Check 1 — the core regression guard: real H264 frames must DECODE over
  // WebRTC in a real browser, with a VIDEO + AUDIO offer (what the real client
  // sends). Run as the SOLE consumer: we are on a non-NDI page so no <NdiVideo>
  // is mounted competing for the pipeline. (Two WebRTC consumers from the SAME
  // host confuse ICE candidate pairing — same IP, only the port differs — and
  // the 2nd gets no media; that is a test-host artifact, not a product bug, so
  // the guard must use a single consumer.)
  //
  // We assert via RTCPeerConnection getStats (framesDecoded / bytesReceived)
  // rather than the <video> element's videoWidth — headless Chrome decodes
  // WebRTC media but does NOT reliably surface dimensions on a <video> bound to
  // a MediaStream, so videoWidth is unreliable in CI. getStats reflects the
  // actual decoder and is the precise measure of the bug.
  //
  // The VIDEO + AUDIO offer is load-bearing, not incidental: the regression
  // that shipped "connected but black" delivered ZERO video frames ONLY when an
  // audio m-line was also negotiated (the per-consumer branch was spliced into
  // the live tee AFTER it was PLAYING, so it never forwarded a buffer). A
  // video-ONLY offer happened to decode frames even on the broken build — which
  // is exactly why the PREVIOUS version of this test was GREEN while every real
  // browser (video + audio) showed black. Verified: broken build → video-only
  // fd=14 (false pass) but video+audio fd=0; fixed build → video+audio fd>0.
  await page.goto(new URL("/", baseURL).toString());
  const result = await page.evaluate(async (sourceId) => {
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
      setTimeout(res, 3000);
    });
    const resp = await fetch(`/ndi/whep/${sourceId}`, {
      method: "POST",
      headers: { "Content-Type": "application/sdp" },
      body: pc.localDescription!.sdp,
    });
    if (!resp.ok) {
      pc.close();
      return {
        ok: false,
        reason: `WHEP POST ${resp.status}`,
        conn: pc.connectionState,
      };
    }
    const location =
      resp.headers.get("Location") || resp.headers.get("location");
    await pc.setRemoteDescription({ type: "answer", sdp: await resp.text() });
    // Poll up to ~25s for decoded frames.
    let inbound: {
      bytes: number;
      framesReceived: number;
      framesDecoded: number;
      frameWidth: number;
      frameHeight: number;
    } | null = null;
    for (let i = 0; i < 50; i++) {
      await new Promise((r) => setTimeout(r, 500));
      (await pc.getStats()).forEach((s) => {
        if (s.type === "inbound-rtp" && s.kind === "video") {
          inbound = {
            bytes: s.bytesReceived,
            framesReceived: s.framesReceived,
            framesDecoded: s.framesDecoded,
            frameWidth: s.frameWidth,
            frameHeight: s.frameHeight,
          };
        }
      });
      if (inbound && inbound.framesDecoded > 0) break;
    }
    const conn = pc.connectionState;
    pc.close();
    // Release the server-side session so check 2's WASM client is the sole
    // consumer (and we don't leak a session on the shared synthetic pipeline).
    if (location) {
      const url = location.startsWith("http")
        ? location
        : new URL(location, document.baseURI).toString();
      try {
        await fetch(url, { method: "DELETE" });
      } catch {
        /* idempotent best-effort */
      }
    }
    return { ok: !!inbound && inbound.framesDecoded > 0, conn, inbound };
  }, src.id);

  expect(
    result.ok,
    `NDI WebRTC must deliver decodable frames (video+audio) — connectionState=${result.conn}, ` +
      `inbound=${JSON.stringify(result.inbound ?? result.reason)}`,
  ).toBe(true);
  expect(result.conn).toBe("connected");
  expect(result.inbound!.framesDecoded).toBeGreaterThan(0);
  expect(result.inbound!.bytes).toBeGreaterThan(0);

  // The synthetic source publishes 2560×1440 (1440p) — the pipeline MUST
  // downscale it to a stage-display-safe resolution before encoding. If it
  // doesn't, the browser can't decode the high-level stream (the bug above
  // would re-trigger: framesDecoded stays 0). Assert the decoded frame is
  // actually downscaled (≤1280 wide) so a regression that drops the
  // videoscale step is caught even if some browser tolerates the high level.
  expect(
    result.inbound!.frameWidth,
    `decoded frame must be downscaled ≤1280 wide, got ${result.inbound!.frameWidth}×${result.inbound!.frameHeight}`,
  ).toBeLessThanOrEqual(1280);

  // Confirm check 1's session is fully released before check 2, so the WASM
  // client is genuinely the SOLE consumer (two consumers from this one test
  // host hit the same-host ICE-pairing artifact). The DELETE above is
  // best-effort + not awaited inside the page, so poll the server snapshot.
  await expect
    .poll(
      async () => {
        const snap = await (
          await request.get(
            new URL(`/ndi/snapshot/${src.id}`, baseURL).toString(),
          )
        ).json();
        return (snap.sessions ?? []).length;
      },
      {
        timeout: 15_000,
        message: "check 1's WHEP session must be released before check 2",
      },
    )
    .toBe(0);

  // ── Check 2 — the REAL stage client path: mount the ndi-fullscreen layout so
  // the WASM <NdiVideo> component does its own connect_whep, and confirm its
  // <video> actually DECODES frames. Now the SOLE consumer (check 1's session
  // was DELETEd above).
  //
  // This MUST assert framesDecoded > 0, not merely connectionState=connected:
  // the #372 bug was that the WASM client used the default ("balanced") bundle
  // policy while the server's webrtcbin is max-bundle, so the transports never
  // lined up — every stage display reached `connected` but received ZERO RTP
  // (black). The OLD version of this check only asserted "connected", so it was
  // GREEN while every real stage display was black. We hook RTCPeerConnection
  // before loading /stage so we can read the WASM client's own getStats.
  await page.addInitScript(() => {
    // @ts-expect-error test-only global
    window.__pcs = [];
    const Orig = window.RTCPeerConnection;
    // @ts-expect-error wrap constructor to capture every PC the WASM creates
    window.RTCPeerConnection = function (...args: unknown[]) {
      // @ts-expect-error spread into native ctor
      const pc = new Orig(...args);
      // @ts-expect-error test-only global
      window.__pcs.push(pc);
      return pc;
    };
    window.RTCPeerConnection.prototype = Orig.prototype;
  });
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });
  await expect(page.locator('[data-role="ndi-video"]')).toHaveCount(1);
  // The WASM client's WHEP session must reach connectionState=connected …
  await expect
    .poll(
      async () => {
        const snap = await (
          await request.get(
            new URL(`/ndi/snapshot/${src.id}`, baseURL).toString(),
          )
        ).json();
        return (snap.sessions ?? []).some(
          (s: { connectionState: string }) => s.connectionState === "connected",
        );
      },
      {
        timeout: 30_000,
        message:
          "the WASM stage client's WHEP session must reach connectionState=connected",
      },
    )
    .toBe(true);
  // … AND the WASM client's <video> must actually DECODE frames (the #372 guard).
  await expect
    .poll(
      async () =>
        page.evaluate(async () => {
          // @ts-expect-error test-only global
          const pcs: RTCPeerConnection[] = window.__pcs || [];
          let best = 0;
          for (const pc of pcs) {
            const stats = await pc.getStats();
            stats.forEach((s) => {
              if (s.type === "inbound-rtp" && s.kind === "video") {
                best = Math.max(best, s.framesDecoded || 0);
              }
            });
          }
          return best;
        }),
      {
        timeout: 30_000,
        message:
          "the WASM stage client must DECODE video frames (framesDecoded > 0); " +
          "connected-but-zero-frames is the #372 max-bundle regression (black stage)",
      },
    )
    .toBeGreaterThan(0);

  // Cleanup.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});
