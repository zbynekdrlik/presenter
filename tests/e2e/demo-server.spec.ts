import { test, expect } from '@playwright/test';
import os from 'node:os';

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
});
