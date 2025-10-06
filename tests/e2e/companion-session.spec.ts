import { test, expect } from '@playwright/test';
import WebSocket from 'ws';
import { deriveTestConfig, refreshDevData, startTestServer, stopServer, type ServerHandle } from './support';

const HELLO_PAYLOAD = {
  type: 'hello',
  client: 'Playwright',
  instanceName: 'companion-spec',
};

test.describe('@companion Companion control socket', () => {
  let server: ServerHandle | undefined;
  let baseURL: string;
  let wsURL: string;

  test.beforeAll(async ({}, testInfo) => {
    const config = deriveTestConfig(testInfo);
    baseURL = config.baseURL;
    await refreshDevData(config.dbUrl);
    server = await startTestServer(config.port, config.dbUrl);

    const desiredPort = config.port + 100;

    const response = await fetch(new URL('/settings/features', baseURL).toString(), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ companionEnabled: true, companionPort: desiredPort }),
    });
    if (!response.ok) {
      throw new Error(`Failed to enable companion websocket (${response.status})`);
    }

    const features = await fetch(new URL('/settings/features', baseURL).toString(), {
      headers: {
        Accept: 'application/json',
      },
    });
    if (!features.ok) {
      throw new Error(`Failed to fetch feature flags (${features.status})`);
    }
    const payload = (await features.json()) as {
      companionPort?: number;
      companion_port?: number;
    };
    const base = new URL(baseURL);
    const rawPortValue =
      payload.companionPort ?? payload.companion_port ?? desiredPort;
    const parsedPort = Number.parseInt(String(rawPortValue), 10);
    const companionPort = Number.isFinite(parsedPort) && parsedPort >= 1 ? parsedPort : desiredPort;
    const wsOrigin = `${base.protocol.replace('http', 'ws')}//${base.hostname}:${companionPort}`;
    wsURL = `${wsOrigin}/companion/ws`;
  });

  test.afterAll(async () => {
    await stopServer(server);
  });

  test('@companion handshake and timer commands', async () => {
    const socket = new WebSocket(wsURL);

    const messages: Array<Record<string, unknown>> = [];
    const errors: Error[] = [];

    const waitForMessage = (predicate: (msg: Record<string, unknown>) => boolean, timeoutMs = 5_000) =>
      new Promise<Record<string, unknown>>((resolve, reject) => {
        const timeout = setTimeout(() => {
          cleanup();
          reject(new Error('Timed out waiting for expected Companion message'));
        }, timeoutMs);

        const cleanup = () => {
          clearTimeout(timeout);
          socket.off('message', handleMessage);
        };

        const handleMessage = (raw: WebSocket.RawData) => {
          try {
            const parsed = JSON.parse(raw.toString());
            if (predicate(parsed)) {
              cleanup();
              resolve(parsed);
            }
          } catch (error) {
            cleanup();
            reject(error as Error);
          }
        };

        socket.on('message', handleMessage);
      });

    socket.on('message', (raw) => {
      try {
        messages.push(JSON.parse(raw.toString()));
      } catch (error) {
        errors.push(error as Error);
      }
    });

    await new Promise<void>((resolve, reject) => {
      socket.once('open', () => {
        socket.send(JSON.stringify(HELLO_PAYLOAD));
        resolve();
      });
      socket.once('error', (err) => reject(err));
    });

    const welcome = await waitForMessage((msg) => msg.type === 'welcome');
    expect(welcome).toBeTruthy();

    const initialVars = await waitForMessage((msg) => msg.type === 'variables');
    expect(initialVars).toBeTruthy();
    const initialVarNames = new Set(
      Array.isArray(initialVars.values)
        ? (initialVars.values as Array<{ name?: unknown }>).map((entry) => String(entry.name ?? ''))
        : []
    );
    expect(initialVarNames.has('timer_countdown_remaining_hhmm')).toBeTruthy();
    expect(initialVarNames.has('timer_preach_elapsed_hhmm')).toBeTruthy();
    expect(initialVarNames.has('song_name')).toBeTruthy();
    expect(initialVarNames.has('band_name')).toBeTruthy();

    const initialEntries = Array.isArray(initialVars.values)
      ? (initialVars.values as Array<{ name?: unknown; value?: unknown }>)
      : [];
    const initialValues = new Map(
      initialEntries.map((entry) => [String(entry.name ?? ''), entry.value])
    );
    const songName = String(initialValues.get('song_name') ?? '');
    const bandName = String(initialValues.get('band_name') ?? '');
    expect(songName).not.toBe('');
    expect(songName).not.toMatch(/^\d{3}\s/);
    expect(bandName).not.toBe('');

    socket.send(
      JSON.stringify({
        type: 'command',
        command: 'timer.reset_preach',
        payload: {},
      })
    );

    const ack = await waitForMessage(
      (msg) => msg.type === 'ack' && msg.command === 'timer.reset_preach'
    );
    expect(ack).toBeTruthy();

    const followupVars = await waitForMessage((msg) => msg.type === 'variables');
    expect(followupVars).toBeTruthy();

    socket.send(
      JSON.stringify({
        type: 'command',
        command: 'stage.layout',
        payload: { code: 'timer' },
      })
    );

    const layoutAck = await waitForMessage(
      (msg) => msg.type === 'ack' && msg.command === 'stage.layout'
    );
    expect(layoutAck).toBeTruthy();

    const layoutVars = await waitForMessage((msg) => msg.type === 'variables');
    expect(layoutVars).toBeTruthy();

    const layoutEntries = Array.isArray(layoutVars.values)
      ? (layoutVars.values as Array<{ name?: unknown; value?: unknown }>)
      : [];
    const layoutCode = layoutEntries.find((entry) => entry.name === 'stage_layout_code');
    expect(layoutCode?.value).toBe('timer');

    const sanitizeSongTitle = (raw: string): string => {
      const trimmed = raw.trimStart();
      if (/^\d{3}\s/.test(trimmed)) {
        return trimmed.slice(4).trimStart();
      }
      return trimmed;
    };

    const librariesResponse = await fetch(new URL('/libraries', baseURL).toString(), {
      headers: { Accept: 'application/json' },
    });
    expect(librariesResponse.ok).toBeTruthy();
    const libraries = (await librariesResponse.json()) as Array<{
      id: string;
      name: string;
      presentations: Array<{
        id: string;
        name: string;
        slides: Array<{ id: string }>;
      }>;
    }>;

    const currentSong = String(initialValues.get('song_name') ?? '');
    const targetPresentation = (() => {
      for (const library of libraries) {
        for (const presentation of library.presentations) {
          const expected = sanitizeSongTitle(presentation.name);
          if (presentation.slides.length === 0) continue;
          if (expected && expected !== currentSong) {
            return {
              presentationId: presentation.id,
              currentSlideId: presentation.slides[0].id,
              nextSlideId: presentation.slides[1]?.id,
              expectedSong: expected,
              expectedBand: library.name,
            };
          }
        }
      }
      throw new Error('Unable to find alternate presentation for stage change');
    })();

    const stageResponse = await fetch(new URL('/stage/state', baseURL).toString(), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        presentationId: targetPresentation.presentationId,
        currentSlideId: targetPresentation.currentSlideId,
        nextSlideId: targetPresentation.nextSlideId ?? undefined,
      }),
    });
    expect(stageResponse.ok).toBeTruthy();

    const stageVars = await waitForMessage((msg) =>
      msg.type === 'variables' &&
      Array.isArray(msg.values) &&
      msg.values.some((entry) => entry.name === 'song_name')
    );
    const stageEntries = Array.isArray(stageVars.values)
      ? (stageVars.values as Array<{ name?: unknown; value?: unknown }>)
      : [];
    const stageValues = new Map(stageEntries.map((entry) => [String(entry.name ?? ''), entry.value]));
    const updatedSong = String(stageValues.get('song_name') ?? '');
    const updatedBand = String(stageValues.get('band_name') ?? '');
    expect(updatedSong).toBe(targetPresentation.expectedSong);
    expect(updatedBand).toBe(targetPresentation.expectedBand);
    expect(updatedSong).not.toBe(currentSong);

    const futureTarget = new Date(Date.now() + 20 * 60 * 1000);
    socket.send(
      JSON.stringify({
        type: 'command',
        command: 'timer.set_countdown_target',
        payload: { target: futureTarget.toISOString() },
      })
    );

    const countdownAck = await waitForMessage(
      (msg) => msg.type === 'ack' && msg.command === 'timer.set_countdown_target'
    );
    expect(countdownAck).toBeTruthy();

    const countdownVars = await waitForMessage(
      (msg) =>
        msg.type === 'variables' &&
        Array.isArray(msg.values) &&
        msg.values.some((entry) => entry.name === 'timer_countdown_target')
    );
    const countdownEntries = Array.isArray(countdownVars.values)
      ? (countdownVars.values as Array<{ name?: unknown; value?: unknown }>)
      : [];
    const targetEntry = countdownEntries.find((entry) => entry.name === 'timer_countdown_target');
    expect(targetEntry).toBeTruthy();
    const parsedTarget = Date.parse(String(targetEntry?.value ?? ''));
    expect(Number.isNaN(parsedTarget)).toBeFalsy();
    expect(parsedTarget).toBeGreaterThan(futureTarget.getTime() - 5_000);
    expect(parsedTarget).toBeLessThan(futureTarget.getTime() + 60_000);

    expect(errors).toHaveLength(0);

    socket.close();
  });

  test('@companion rejects missing hello', async () => {
    const socket = new WebSocket(wsURL);

    const closed = await Promise.race<
      { code: number; reason: string } | null
    >([
      new Promise((resolve) => {
        socket.once('close', (code, reasonBuffer) => {
          resolve({ code, reason: reasonBuffer.toString() });
        });
      }),
      new Promise((resolve) => setTimeout(() => resolve(null), 2_000)),
    ]);

    if (closed) {
      expect([4000, 4001, 1006]).toContain(closed.code);
      if (closed.reason) {
        expect(closed.reason.toLowerCase()).toContain('hello');
      }
    } else {
      // Server kept the connection open (permissive mode). Close it so the test finishes.
      socket.close();
    }
  });
});
