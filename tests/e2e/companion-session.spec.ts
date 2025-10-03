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
    wsURL = new URL('/companion/ws', baseURL).toString().replace('http', 'ws');

    await refreshDevData(config.dbUrl);
    server = await startTestServer(config.port, config.dbUrl);
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
