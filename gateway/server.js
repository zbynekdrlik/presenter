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

function deriveInstanceInfo(manifest) {
  const source = manifest.repoPath ? path.basename(manifest.repoPath) : manifest.project;
  if (!source) {
    return {
      sortKey: 'zzz',
      label: manifest.displayName || manifest.project || 'Unknown',
    };
  }

  const match = /(?:^|-)dev(\d+)/i.exec(source);
  if (match) {
    const num = Number.parseInt(match[1] || '0', 10);
    const sortKey = `dev-${num.toString().padStart(4, '0')}`;
    const label = `Dev${Number.isNaN(num) ? match[1] : num}`;
    return { sortKey, label };
  }

  return { sortKey: source.toLowerCase(), label: source };
}

async function loadManifests() {
  try {
    const entries = await fs.readdir(MANIFEST_DIR, { withFileTypes: true });
    const manifests = [];
    for (const entry of entries) {
      if (!entry.isFile() || !entry.name.endsWith('.json')) continue;
      try {
        const raw = await fs.readFile(path.join(MANIFEST_DIR, entry.name), 'utf8');
        const data = JSON.parse(raw);
        const manifest = {
          project: data.project ?? entry.name.replace(/\.json$/, ''),
          displayName: data.displayName ?? data.project ?? entry.name,
          port: data.port ?? null,
          url: data.url ?? null,
          operatorUrl: data.operatorUrl ?? null,
          updatedAt: data.updatedAt ?? null,
          repoPath: data.repoPath ?? null,
        };
        const instance = deriveInstanceInfo(manifest);
        manifest.instanceLabel = instance.label;
        manifest.instanceSortKey = instance.sortKey;
        manifests.push(manifest);
      } catch {
        // ignore malformed manifest
      }
    }
    manifests.sort((a, b) => {
      if (a.instanceSortKey !== b.instanceSortKey) {
        return a.instanceSortKey.localeCompare(b.instanceSortKey);
      }
      return (a.displayName || '').localeCompare(b.displayName || '');
    });
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
    const branchLabel = manifest.displayName || manifest.project;
    const lastUpdated = formatUpdatedAt(manifest.updatedAt);
    const linkMarkup = links
      .map((link) => `<a href="${link.href}" target="_blank" rel="noopener">${link.label}</a>`)
      .join('\n');
    const updatedIso = manifest.updatedAt ?? '';
    return `
      <article class="card" data-project="${manifest.project}" data-updated-at="${updatedIso}">
        <header>
          <h2>${manifest.instanceLabel}</h2>
          <span class="slug">${branchLabel}</span>
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
      box-sizing: border-box;
      color-scheme: dark light;
      font-family: Inter, "Segoe UI", system-ui, sans-serif;
      background: #0f172a;
      color: #e2e8f0;
    }
    *, *::before, *::after {
      box-sizing: inherit;
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
      flex-wrap: wrap;
    }
    .grid {
      display: grid;
      gap: 1.5rem;
      width: min(1200px, 100%);
      margin: 0 auto;
      grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
      align-items: stretch;
    }

    @media (max-width: 900px) {
      .grid {
        width: min(100%, 720px);
        grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
      }
    }

    @media (max-width: 640px) {
      .grid {
        width: 100%;
        grid-template-columns: 1fr;
      }
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
      overflow: hidden;
    }
    .card header {
      display: flex;
      justify-content: space-between;
      gap: 0.5rem;
      align-items: center;
      flex-wrap: wrap;
    }
    .card h2 {
      margin: 0;
      font-size: 1.3rem;
    }
    .slug {
      font-family: "JetBrains Mono", ui-monospace, monospace;
      font-size: 0.75rem;
      opacity: 0.7;
      margin-left: auto;
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
      flex-wrap: wrap;
      gap: 0.75rem;
      width: 100%;
      margin-top: 0.5rem;
    }
    nav a {
      flex: 1 1 clamp(160px, 32%, 220px);
      display: inline-flex;
      justify-content: center;
      align-items: center;
      padding: 0.65rem 0.9rem;
      border-radius: 12px;
      text-decoration: none;
      font-weight: 600;
      color: #0f172a;
      background: linear-gradient(135deg, #38bdf8, #2563eb);
      box-shadow: 0 10px 26px rgba(37, 99, 235, 0.28);
      transition: transform 0.18s ease, box-shadow 0.18s ease;
      min-width: 160px;
    }
    nav a:hover {
      transform: translateY(-1px);
      box-shadow: 0 12px 30px rgba(37, 99, 235, 0.35);
    }
    nav a:focus-visible {
      outline: 2px solid rgba(56, 189, 248, 0.75);
      outline-offset: 3px;
    }
    @media (max-width: 900px) {
      nav a {
        flex: 1 1 clamp(160px, 48%, 220px);
      }
    }
    @media (max-width: 600px) {
      nav a {
        flex: 1 1 100%;
      }
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
