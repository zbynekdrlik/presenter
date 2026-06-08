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
  expect(created.status()).toBeLessThan(500);
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

  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  // First confirm the real client path renders: the stage page mounts the
  // <NdiVideo> component for the active source.
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });
  await expect(page.locator('[data-role="ndi-video"]')).toHaveCount(1);

  // The core regression guard: real H264 frames must DECODE over WebRTC in a
  // real browser. We assert via RTCPeerConnection getStats (framesDecoded /
  // bytesReceived) from a controlled WHEP exchange rather than the <video>
  // element's videoWidth — headless Chrome decodes WebRTC media but does NOT
  // surface dimensions on a <video> bound to a MediaStream, so videoWidth is
  // unreliable in CI. getStats reflects the actual decoder and is the precise
  // measure of the bug: the #336 regression left the connection stuck (ICE
  // never connected / DTLS bundle hung / payload-type mismatch) so
  // framesDecoded stayed 0 forever.
  const result = await page.evaluate(async (sourceId) => {
    const pc = new RTCPeerConnection();
    pc.addTransceiver("video", { direction: "recvonly" });
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
      return { ok: false, reason: `WHEP POST ${resp.status}`, conn: pc.connectionState };
    }
    await pc.setRemoteDescription({ type: "answer", sdp: await resp.text() });
    // Poll up to ~25s for decoded frames.
    let inbound: { bytes: number; framesReceived: number; framesDecoded: number } | null =
      null;
    for (let i = 0; i < 50; i++) {
      await new Promise((r) => setTimeout(r, 500));
      (await pc.getStats()).forEach((s) => {
        if (s.type === "inbound-rtp" && s.kind === "video") {
          inbound = {
            bytes: s.bytesReceived,
            framesReceived: s.framesReceived,
            framesDecoded: s.framesDecoded,
          };
        }
      });
      if (inbound && inbound.framesDecoded > 0) break;
    }
    const conn = pc.connectionState;
    pc.close();
    return { ok: !!inbound && inbound.framesDecoded > 0, conn, inbound };
  }, src.id);

  expect(
    result.ok,
    `NDI WebRTC must deliver decodable frames — connectionState=${result.conn}, ` +
      `inbound=${JSON.stringify(result.inbound ?? result.reason)}`,
  ).toBe(true);
  expect(result.conn).toBe("connected");
  expect(result.inbound!.framesDecoded).toBeGreaterThan(0);
  expect(result.inbound!.bytes).toBeGreaterThan(0);

  // Cleanup.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});
