import { spawn, type ChildProcessWithoutNullStreams } from 'child_process';
import { once } from 'events';
import http from 'http';
import path from 'path';
import type { AddressInfo } from 'net';
import type { TestInfo } from '@playwright/test';

export const REPO_ROOT = process.cwd();

function stableHash(input: string): number {
  let hash = 0;
  for (let i = 0; i < input.length; i += 1) {
    hash = (hash * 31 + input.charCodeAt(i)) >>> 0;
  }
  return hash;
}

export type ServerHandle = {
  process: ChildProcessWithoutNullStreams;
  port: number;
  stop: () => Promise<void>;
};

export type MockResolumeHandle = {
  port: number;
  setOnline: (online: boolean) => void;
  close: () => Promise<void>;
};

export type TestConfig = {
  workerIndex: number;
  port: number;
  dbUrl: string;
  baseURL: string;
  oscPort: number;
};

export function deriveTestConfig(testInfo: TestInfo): TestConfig {
  const workerIndex = testInfo.workerIndex ?? 0;
  const basePort = Number.parseInt(process.env.PRESENTER_PORT_BASE ?? '18999', 10);
  const scopeKey = testInfo.file ?? testInfo.title ?? `worker-${workerIndex}`;
  const fileOffset = stableHash(scopeKey) % 50;
  const port = basePort + workerIndex * 100 + fileOffset;
  const explicitDbUrl = process.env.PRESENTER_DB_URL;
  const defaultDbPath = path.join(REPO_ROOT, 'var', 'tmp', `presenter_e2e_${workerIndex}.db`);
  const dbUrl = explicitDbUrl ?? `sqlite://${defaultDbPath}`;
  const baseURL = `http://127.0.0.1:${port}`;
  const oscPort = port + 1;
  return { workerIndex, port, dbUrl, baseURL, oscPort };
}

export function runShell(command: string, extraEnv: NodeJS.ProcessEnv = {}): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn('bash', ['-lc', command], {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        ...extraEnv,
      },
      stdio: 'inherit',
    });

    child.on('error', reject);
    child.on('exit', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`Command failed (${code ?? 'unknown'}): ${command}`));
      }
    });
  });
}

const DEFAULT_LIBRARY_ROOT =
  process.env.PRESENTER_LIBRARY_ROOT ??
  path.resolve(REPO_ROOT, '..', 'presenter-libraries');

export async function refreshDevData(dbUrl: string, root = DEFAULT_LIBRARY_ROOT) {
  await runShell(`PRESENTER_DB_URL=${dbUrl} ./scripts/dev/refresh-dev-data.sh "${root}"`, {
    PRESENTER_DB_URL: dbUrl,
  });
}

async function waitForServerReady(baseURL: string, timeoutMs = 240_000) {
  const startedAt = Date.now();
  const healthUrl = new URL('/healthz', baseURL).toString();

  while (Date.now() - startedAt < timeoutMs) {
    try {
      const response = await fetch(healthUrl, { signal: AbortSignal.timeout(5_000) });
      if (response.ok) {
        return;
      }
    } catch {
      // retry after delay
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }

  throw new Error(`Presenter server did not become ready within ${timeoutMs}ms`);
}

export async function startTestServer(port: number, dbUrl: string, oscPort?: number): Promise<ServerHandle> {
  const env = {
    ...process.env,
    PRESENTER_DB_URL: dbUrl,
    PRESENTER_PORT: String(port),
    ...(oscPort ? { PRESENTER_OSC_LISTEN_PORT: String(oscPort) } : {}),
    PRESENTER_ANDROID_ADB_BIN: process.env.PRESENTER_ANDROID_ADB_BIN ?? 'true',
    RUST_LOG: process.env.RUST_LOG ?? 'presenter_server=info,tower_http=warn,sqlx=warn',
  };

  const processHandle = spawn(
    'bash',
    [
      '-lc',
      `PRESENTER_DB_URL=${dbUrl} PRESENTER_PORT=${port} ${oscPort ? `PRESENTER_OSC_LISTEN_PORT=${oscPort} ` : ''}cargo run -p presenter-server`,
    ],
    {
      cwd: REPO_ROOT,
      env,
      stdio: 'inherit',
    }
  );

  await waitForServerReady(`http://127.0.0.1:${port}`);

  return {
    process: processHandle,
    port,
    stop: async () => {
      processHandle.kill('SIGTERM');
      await once(processHandle, 'exit');
    },
  };
}

export async function stopServer(handle?: ServerHandle) {
  if (!handle) return;
  handle.process.kill('SIGTERM');
  await once(handle.process, 'exit');
}

export async function startMockResolume(): Promise<MockResolumeHandle> {
  let online = true;

  const server = http.createServer((req, res) => {
    const { method, url } = req;
    if (!url) {
      res.statusCode = 400;
      return res.end('bad request');
    }

    if (!online) {
      res.statusCode = 503;
      return res.end('resolume offline');
    }

    if (method === 'GET' && url.startsWith('/api/v1/composition')) {
      res.writeHead(200, { 'content-type': 'application/json' });
      const body = {
        layers: [
          {
            clips: [
              clip(100, '#main-a', 1),
              clip(101, '#main-b', 2),
              clip(200, '#translate-a', 10),
              clip(201, '#translate-b', 20),
              clip(300, '#bible-a', 30),
              clip(301, '#bible-b', 31),
              clip(400, '#bible-translate-a', 40),
              clip(401, '#bible-translate-b', 41),
              clip(500, '#bible-clear', undefined),
              clip(600, '#timer', 60),
              clip(700, '#song-name', undefined),
              clip(701, '#band-name', undefined),
            ],
          },
        ],
      };
      const payload = JSON.stringify(body);
      res.end(payload);
      return;
    }

    if (method === 'PUT' && url.startsWith('/api/v1/parameter/by-id/')) {
      res.statusCode = 200;
      res.end();
      return;
    }

    if (method === 'POST' && url.startsWith('/api/v1/composition/clips/by-id/')) {
      res.statusCode = 200;
      res.end();
      return;
    }

    res.statusCode = 404;
    res.end('not found');
  });

  function clip(id: number, name: string, param?: number) {
    const sourceparams = param
      ? {
          text: {
            valuetype: 'ParamText',
            id: param,
          },
        }
      : {};

    return {
      id,
      name: { value: name },
      video: { sourceparams },
    };
  }

  await new Promise<void>((resolve, reject) => {
    server.listen(0, '127.0.0.1', (err?: Error) => {
      if (err) reject(err);
      else resolve();
    });
  });

  const address = server.address() as AddressInfo;

  return {
    port: address.port,
    setOnline: (value: boolean) => {
      online = value;
    },
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((err) => {
          if (err) reject(err);
          else resolve();
        });
      }),
  };
}
