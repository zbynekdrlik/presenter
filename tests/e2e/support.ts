import { spawn, type ChildProcessWithoutNullStreams } from 'child_process';
import { once } from 'events';
import type { TestInfo } from '@playwright/test';

export const REPO_ROOT = process.cwd();

export type ServerHandle = {
  process: ChildProcessWithoutNullStreams;
  port: number;
  stop: () => Promise<void>;
};

export function deriveTestConfig(testInfo: TestInfo) {
  const workerIndex = testInfo.workerIndex ?? 0;
  const basePort = Number.parseInt(process.env.PRESENTER_PORT ?? '8899', 10);
  const port = basePort + workerIndex;
  const explicitDbUrl = process.env.PRESENTER_DB_URL;
  const dbUrl = explicitDbUrl ?? `sqlite://presenter_e2e_${workerIndex}.db`;
  const baseURL = `http://127.0.0.1:${port}`;
  return { workerIndex, port, dbUrl, baseURL };
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

export async function refreshDevData(dbUrl: string, root = 'Propresenter library') {
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

export async function startTestServer(port: number, dbUrl: string): Promise<ServerHandle> {
  const env = {
    ...process.env,
    PRESENTER_DB_URL: dbUrl,
    PRESENTER_PORT: String(port),
    RUST_LOG: process.env.RUST_LOG ?? 'presenter_server=info,tower_http=warn,sqlx=warn',
  };

  const processHandle = spawn(
    'bash',
    ['-lc', `PRESENTER_DB_URL=${dbUrl} PRESENTER_PORT=${port} cargo run -p presenter-server`],
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
