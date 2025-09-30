import { test, expect } from '@playwright/test';
import os from 'node:os';
import path from 'node:path';

const STALE_THRESHOLD_MS = Number(process.env.PRESENTER_DEMO_STALE_MS ?? 30 * 60 * 1000);
const EXPECTED_PROJECT = process.env.PRESENTER_DEMO_PROJECT ?? slugify(path.basename(process.cwd()));

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
  const hosts = getDemoHosts();
  for (const host of hosts) {
    test(`responds on ${host}`, async ({ request }) => {
      const response = await request.get(`http://${host}/healthz`, {
        timeout: 15_000,
      });
      expect(response.ok(), `Failed to reach demo server on ${host}:80`).toBeTruthy();
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
