#!/usr/bin/env node
import { promises as fs } from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(process.cwd(), 'ops/companion');
const BLUEPRINT_PATH = path.join(ROOT, 'presenter-companion-profile.json');
const OUTPUT_DIR = path.join(ROOT, 'generated');
const OUTPUT_PATH = path.join(OUTPUT_DIR, 'presenter-companion-profile.export.json');

async function main() {
  const blueprintRaw = await fs.readFile(BLUEPRINT_PATH, 'utf8');
  const blueprint = JSON.parse(blueprintRaw);

  const exportPayload = {
    $schema: 'https://presenter.dev/schemas/companion-profile-blueprint.json',
    generatedAt: new Date().toISOString(),
    blueprint,
  };

  await fs.mkdir(OUTPUT_DIR, { recursive: true });
  await fs.writeFile(OUTPUT_PATH, JSON.stringify(exportPayload, null, 2));

  console.log('Companion blueprint rendered to', OUTPUT_PATH);
  console.log('Import the file into Companion via Connections → Import, then adjust host/port/token as needed.');
}

main().catch((error) => {
  console.error('Failed to generate Companion profile export:', error);
  process.exitCode = 1;
});
