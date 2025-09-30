import express from 'express';
import fs from 'fs/promises';
import path from 'path';

const app = express();
const PORT = Number(process.env.PORT || 8080);
const MANIFEST_DIR = process.env.DEMO_MANIFEST_DIR || '/manifests';
const REFRESH_INTERVAL_MS = Number(process.env.REFRESH_INTERVAL_MS || 3000);
const DATE_FORMATTER = new Intl.DateTimeFormat('sk-SK', {
  dateStyle: 'short',
  timeStyle: 'medium',
  timeZone: 'Europe/Bratislava',
});

function formatUpdatedAt(value) {
  if (!value) return '—';
  try {
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) {
      return value;
    }
    return DATE_FORMATTER.format(parsed);
  } catch (error) {
    return value;
  }
}

app.get('/healthz', (_req, res) => {
  res.json({ status: 'ok' });
});

async function loadManifests() {
  try {
    const entries = await fs.readdir(MANIFEST_DIR, { withFileTypes: true });
    const manifests = [];
    for (const entry of entries) {
      if (!entry.isFile() || !entry.name.endsWith('.json')) continue;
      try {
        const raw = await fs.readFile(path.join(MANIFEST_DIR, entry.name), 'utf8');
       const data = JSON.parse(raw);
       manifests.push({
         project: data.project ?? entry.name.replace(/\.json$/, ''),
         displayName: data.displayName ?? data.project ?? entry.name,
         port: data.port ?? null,
         url: data.url ?? null,
         operatorUrl: data.operatorUrl ?? null,
          updatedAt: data.updatedAt ?? null,
          repoPath: data.repoPath ?? null,
        });
      } catch {
        // ignore malformed manifest
      }
    }
    manifests.sort((a, b) => (a.displayName || '').localeCompare(b.displayName || ''));
    return manifests;
  } catch (error) {
    if (error.code === 'ENOENT') {
      return [];
    }
    throw error;
  }
}

function render(manifests, baseOrigin) {
  const originUrl = new URL(baseOrigin);
  const rows = manifests.map((manifest) => {
    const demoUrl = manifest.port
      ? `${originUrl.protocol}//${originUrl.hostname}:${manifest.port}/`
      : '#';
    const url = manifest.url || demoUrl;
    const operator = manifest.operatorUrl || (manifest.port ? `${url}ui/operator` : '#');
    const repoPath = manifest.repoPath || manifest.project;
    const tablet = manifest.port ? `${url}ui/tablet` : '#';
    const bible = manifest.port ? `${url}ui/bible` : '#';
    const stageSnv = manifest.port ? `${url}stage/worship-snv` : '#';
    const stagePp = manifest.port ? `${url}stage/worship-pp` : '#';
    const stageTimer = manifest.port ? `${url}stage/timer` : '#';
    const stagePreach = manifest.port ? `${url}stage/preach` : '#';
    const links = [
      { label: 'Open Demo', href: url },
      { label: 'Operator UI', href: operator },
      { label: 'Tablet UI', href: tablet },
      { label: 'Bible UI', href: bible },
      { label: 'Stage SNV', href: stageSnv },
      { label: 'Stage PP', href: stagePp },
      { label: 'Stage Timer', href: stageTimer },
      { label: 'Stage Preach', href: stagePreach },
    ];
    const lastUpdated = formatUpdatedAt(manifest.updatedAt);
    const linkMarkup = links
      .map((link) => `<a href="${link.href}" target="_blank" rel="noopener">${link.label}</a>`)
      .join('\n');
    return `
      <article class="card">
        <header>
          <h2>${manifest.displayName}</h2>
          <span class="slug">${manifest.project}</span>
        </header>
        <dl>
          <div><dt>Port</dt><dd>${manifest.port ?? '—'}</dd></div>
          <div><dt>Last updated</dt><dd>${lastUpdated}</dd></div>
          <div><dt>Repository</dt><dd><code>${repoPath}</code></dd></div>
        </dl>
        <nav>
          ${linkMarkup}
        </nav>
      </article>`;
  }).join('\n');

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Presenter Demo Gateway</title>
  <meta http-equiv="refresh" content="${Math.max(REFRESH_INTERVAL_MS / 1000, 5)}" />
  <style>
    :root {
      color-scheme: dark light;
      font-family: Inter, "Segoe UI", system-ui, sans-serif;
      background: #0f172a;
      color: #e2e8f0;
    }
    body {
      margin: 0;
      padding: 2rem;
      display: flex;
      flex-direction: column;
      gap: 2rem;
      min-height: 100vh;
    }
    header.page {
      display: flex;
      justify-content: space-between;
      align-items: baseline;
      gap: 1rem;
    }
    .grid {
      display: grid;
      gap: 1.5rem;
      grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    }
    .card {
      background: rgba(15, 23, 42, 0.75);
      border: 1px solid rgba(148, 163, 184, 0.25);
      border-radius: 16px;
      padding: 1.25rem;
      display: flex;
      flex-direction: column;
      gap: 1rem;
      box-shadow: 0 16px 40px rgba(15, 23, 42, 0.35);
    }
    .card header {
      display: flex;
      justify-content: space-between;
      gap: 0.5rem;
      align-items: baseline;
    }
    .card h2 {
      margin: 0;
      font-size: 1.3rem;
    }
    .slug {
      font-family: "JetBrains Mono", ui-monospace, monospace;
      font-size: 0.75rem;
      opacity: 0.7;
    }
    dl {
      display: grid;
      grid-template-columns: auto 1fr;
      gap: 0.3rem 1rem;
      margin: 0;
    }
    dt {
      font-weight: 600;
      opacity: 0.7;
    }
    dd {
      margin: 0;
    }
    nav {
      display: flex;
      gap: 0.75rem;
    }
    nav a {
      flex: 1 1 auto;
      text-align: center;
      padding: 0.6rem 0.8rem;
      border-radius: 12px;
      text-decoration: none;
      font-weight: 600;
      color: #0f172a;
      background: #38bdf8;
      transition: background 0.2s ease, transform 0.2s ease;
    }
    nav a:hover {
      background: #0ea5e9;
      transform: translateY(-1px);
    }
    .empty {
      opacity: 0.8;
    }
  </style>
</head>
<body>
  <header class="page">
    <div>
      <h1>Presenter Demo Gateway</h1>
      <p>Active demos are refreshed every ${Math.max(REFRESH_INTERVAL_MS / 1000, 5)} seconds.</p>
    </div>
  </header>
  <section class="grid">
    ${rows || '<p class="empty">No demos are currently running.</p>'}
  </section>
</body>
</html>`;
}

app.get('/', async (req, res, next) => {
  try {
    const manifests = await loadManifests();
    const forwardedProto = req.headers['x-forwarded-proto'];
    const protocol = Array.isArray(forwardedProto)
      ? forwardedProto[0]
      : forwardedProto || req.protocol || 'http';
    const hostHeader = req.headers['x-forwarded-host'] || req.headers.host || 'localhost';
    const host = Array.isArray(hostHeader) ? hostHeader[0] : hostHeader;
    const baseOrigin = `${protocol}://${host}`;
    res.type('html').send(render(manifests, baseOrigin));
  } catch (error) {
    next(error);
  }
});

app.use((err, _req, res, _next) => {
  console.error(err);
  res.status(500).json({ error: 'gateway_error', message: err.message });
});

app.listen(PORT, () => {
  console.log(`Gateway listening on :${PORT}, watching ${MANIFEST_DIR}`);
});
