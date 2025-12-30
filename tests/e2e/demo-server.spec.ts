import './stub-network';
import { test, expect } from '@playwright/test';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { runShell } from './support';

const STALE_THRESHOLD_MS = Number(process.env.PRESENTER_DEMO_STALE_MS ?? 30 * 60 * 1000);
const EXPECTED_PROJECT = process.env.PRESENTER_DEMO_PROJECT ?? slugify(path.basename(process.cwd()));
const DISPLAY_NAME = process.env.PRESENTER_DEMO_DISPLAY_NAME ?? process.env.PRESENTER_BRANCH ?? 'Playwright Demo';

function resolveManifestDir(): string {
  if (process.env.PRESENTER_MANIFEST_DIR) {
    return process.env.PRESENTER_MANIFEST_DIR;
  }

  const stateRoot = process.env.PRESENTER_STATE_DIR
    ?? path.join(process.env.XDG_DATA_HOME ?? path.join(os.homedir(), '.local/share'), 'presenter-demos');

  return path.join(stateRoot, 'manifests');
}

async function manifestAgeMs(): Promise<number | null> {
  const manifestPath = path.join(resolveManifestDir(), `${EXPECTED_PROJECT}.json`);
  try {
    const raw = await fs.readFile(manifestPath, 'utf8');
    const data = JSON.parse(raw);
    if (!data.updatedAt) {
      return null;
    }
    const parsed = new Date(data.updatedAt);
    if (Number.isNaN(parsed.getTime())) {
      return null;
    }
    return Date.now() - parsed.getTime();
  } catch (error: unknown) {
    const code = (error as NodeJS.ErrnoException)?.code;
    if (code === 'ENOENT') {
      return null;
    }
    throw error;
  }
}

function manifestIsFresh(age: number | null): boolean {
  if (age === null) {
    return false;
  }
  return age >= 0 && age <= STALE_THRESHOLD_MS;
}

async function ensureDemoFresh() {
  if (process.env.PRESENTER_SKIP_DEMO_REFRESH === '1') {
    return;
  }
  const age = await manifestAgeMs();
  if (manifestIsFresh(age)) {
    return;
  }
  await runShell(`./scripts/docker/run-demo.sh --name ${EXPECTED_PROJECT} --display-name "${DISPLAY_NAME}"`);

  for (let attempt = 0; attempt < 10; attempt += 1) {
    const refreshedAge = await manifestAgeMs();
    if (manifestIsFresh(refreshedAge)) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }

  throw new Error('Demo manifest did not refresh after run-demo execution');
}

function slugify(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

function getDemoHosts(): string[] {
  const hosts = new Set<string>(['127.0.0.1', 'localhost']);
  const interfaces = os.networkInterfaces();
  for (const entries of Object.values(interfaces)) {
    if (!entries) continue;
    for (const entry of entries) {
      if (entry && entry.family === 'IPv4' && !entry.internal) {
        const address = entry.address;
        if (!address.startsWith('169.254.')) {
          hosts.add(address);
        }
      }
    }
  }
  const extra = process.env.PRESENTER_DEMO_HOSTS;
  if (extra) {
    extra
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean)
      .forEach((value) => hosts.add(value));
  }
  return Array.from(hosts);
}

test.describe('demo server availability', () => {
  test.beforeAll(async () => {
    await ensureDemoFresh();
  });

  const hosts = getDemoHosts();
  for (const host of hosts) {
    test(`responds on ${host}`, async ({ request }) => {
      await expect(async () => {
        const response = await request.get(`http://${host}/healthz`, {
          timeout: 5_000,
        });
        expect(response.ok(), `Failed to reach demo server on ${host}:80`).toBeTruthy();
      }).toPass({ timeout: 60_000, intervals: [1_000] });
    });
  }

  test('landing page reflects fresh manifest metadata', async ({ page }) => {
    const host = process.env.PRESENTER_DEMO_HOST ?? '127.0.0.1';
    const response = await page.goto(`http://${host}/`, { waitUntil: 'domcontentloaded' });
    expect(response?.ok(), `Failed to load landing page on ${host}`).toBeTruthy();

    const card = await page.$(`[data-project="${EXPECTED_PROJECT}"]`);
    expect(card, `Expected demo card for project ${EXPECTED_PROJECT}`).not.toBeNull();

    const isoTimestamp = await card!.getAttribute('data-updated-at');
    expect(isoTimestamp, 'Card missing data-updated-at attribute').toBeTruthy();

    const parsed = isoTimestamp ? new Date(isoTimestamp) : null;
    expect(parsed && !Number.isNaN(parsed.getTime()), `Invalid timestamp: ${isoTimestamp}`).toBeTruthy();

    const age = Date.now() - (parsed?.getTime() ?? 0);
    expect(age >= 0 && age <= STALE_THRESHOLD_MS, `Manifest for ${EXPECTED_PROJECT} is stale: ${age}ms old`).toBeTruthy();
  });
});
