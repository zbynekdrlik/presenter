#!/usr/bin/env node
import { performance } from 'node:perf_hooks';
import process from 'node:process';

function parseArgs(argv) {
  const args = {
    host: 'resolume.lan',
    port: 8090,
    tokens: ['#main-a'],
    iterations: 10,
    timeout: 200,
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--host' && argv[i + 1]) {
      args.host = argv[++i];
    } else if (arg === '--port' && argv[i + 1]) {
      args.port = Number.parseInt(argv[++i], 10);
    } else if (arg === '--tokens' && argv[i + 1]) {
      args.tokens = argv[++i]
        .split(',')
        .map((token) => token.trim())
        .filter(Boolean);
    } else if (arg === '--iterations' && argv[i + 1]) {
      args.iterations = Number.parseInt(argv[++i], 10);
    } else if (arg === '--timeout' && argv[i + 1]) {
      args.timeout = Number.parseInt(argv[++i], 10);
    } else if (arg === '--help' || arg === '-h') {
      printUsage();
      process.exit(0);
    } else {
      console.error(`Unknown option: ${arg}`);
      printUsage();
      process.exit(1);
    }
  }

  if (args.iterations <= 0) {
    throw new Error('Iterations must be greater than zero.');
  }
  return args;
}

function printUsage() {
  console.log(`Usage: profile-resolume-latency [options]
  --host HOST            Resolume host (default: resolume.lan)
  --port PORT            Resolume port (default: 8090)
  --tokens LIST          Comma-separated clip tokens (default: "#main-a")
  --iterations N         Number of measurement iterations (default: 10)
  --timeout MS           Request timeout in milliseconds (default: 200)
`);
}

async function fetchJson(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...(options.headers ?? {}),
    },
  });
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`Request failed (${response.status}): ${body}`);
  }
  return response.json();
}

function buildClipMapping(composition, tokens) {
  const mapping = {};
  const loweredTokens = tokens.map((t) => t.toLowerCase());

  const layers = composition.layers ?? [];
  for (const layer of layers) {
    const clips = layer.clips ?? [];
    for (const clip of clips) {
      const name = clip?.name?.value;
      if (typeof name !== 'string') continue;
      const nameLower = name.toLowerCase();
      const token = loweredTokens.find((candidate) => nameLower.includes(candidate));
      if (!token) continue;

      const textParamId = extractTextParamId(clip);
      if (!textParamId) continue;
      mapping[token] = {
        clipId: clip.id,
        textParamId,
      };
    }
  }

  const missing = loweredTokens.filter((token) => !mapping[token]);
  if (missing.length) {
    throw new Error(`Missing clips for tokens: ${missing.join(', ')}`);
  }
  return mapping;
}

function extractTextParamId(clip) {
  const params = clip?.video?.sourceparams;
  if (!params) return null;
  if (Array.isArray(params)) {
    for (const param of params) {
      const id = parseParam(param);
      if (id) return id;
    }
    return null;
  }
  if (typeof params === 'object') {
    for (const value of Object.values(params)) {
      const id = parseParam(value);
      if (id) return id;
    }
  }
  return null;
}

function parseParam(candidate) {
  if (!candidate || typeof candidate !== 'object') return null;
  if (typeof candidate.valuetype !== 'string') return null;
  if (candidate.valuetype.toLowerCase() !== 'paramtext') return null;
  return typeof candidate.id === 'number' ? candidate.id : null;
}

function computeStats(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const sum = values.reduce((acc, value) => acc + value, 0);
  const avg = sum / values.length;
  const p95 = percentile(sorted, 0.95);
  return {
    min: sorted[0],
    max: sorted[sorted.length - 1],
    avg,
    p95,
  };
}

function percentile(sortedValues, percentile) {
  if (sortedValues.length === 0) return 0;
  const idx = Math.min(sortedValues.length - 1, Math.floor(percentile * sortedValues.length));
  return sortedValues[idx];
}

async function measureToken(host, port, token, target, iterations, timeoutMs) {
  const updateDurations = [];
  const triggerDurations = [];
  const totalDurations = [];
  const baseUrl = `http://${host}:${port}/api/v1`;

  for (let i = 0; i < iterations; i++) {
    const payload = { value: `Latency probe ${token} ${Date.now()}-${Math.random().toString(36).slice(2)}` };
    const updateUrl = `${baseUrl}/parameter/by-id/${target.textParamId}`;
    const triggerUrl = `${baseUrl}/composition/clips/by-id/${target.clipId}/connect`;

    const updateStart = performance.now();
    await fetch(updateUrl, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      keepalive: false,
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const updateEnd = performance.now();

    const triggerStart = performance.now();
    await fetch(triggerUrl, {
      method: 'POST',
      keepalive: false,
      signal: AbortSignal.timeout(timeoutMs),
    });
    const triggerEnd = performance.now();

    updateDurations.push(updateEnd - updateStart);
    triggerDurations.push(triggerEnd - triggerStart);
    totalDurations.push(triggerEnd - updateStart);
  }

  return {
    update: computeStats(updateDurations),
    trigger: computeStats(triggerDurations),
    total: computeStats(totalDurations),
  };
}

async function main() {
  try {
    const args = parseArgs(process.argv);
    const tokens = args.tokens;
    const compositionUrl = `http://${args.host}:${args.port}/api/v1/composition`;
    console.log(`[profile] Fetching composition from ${compositionUrl}`);
    const composition = await fetchJson(compositionUrl, {
      signal: AbortSignal.timeout(args.timeout * 10),
    });

    const mapping = buildClipMapping(composition, tokens);
    for (const token of tokens) {
      console.log(`[profile] Measuring token ${token}`);
      const stats = await measureToken(
        args.host,
        args.port,
        token,
        mapping[token.toLowerCase()],
        args.iterations,
        args.timeout
      );
      console.log(`  Update ms    → min ${stats.update.min.toFixed(1)} | avg ${stats.update.avg.toFixed(1)} | p95 ${stats.update.p95.toFixed(1)} | max ${stats.update.max.toFixed(1)}`);
      console.log(`  Trigger ms   → min ${stats.trigger.min.toFixed(1)} | avg ${stats.trigger.avg.toFixed(1)} | p95 ${stats.trigger.p95.toFixed(1)} | max ${stats.trigger.max.toFixed(1)}`);
      console.log(`  Total ms     → min ${stats.total.min.toFixed(1)} | avg ${stats.total.avg.toFixed(1)} | p95 ${stats.total.p95.toFixed(1)} | max ${stats.total.max.toFixed(1)}`);
    }
  } catch (error) {
    console.error('[profile] Error:', error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

main();
