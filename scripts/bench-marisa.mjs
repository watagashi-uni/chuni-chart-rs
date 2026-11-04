#!/usr/bin/env node
// Compare render time with the original MarisaBot frontend (Vue page + Chromium screenshot).
//
// 1. Build MarisaBot's Marisa.Frontend (`npx vite build`), copy the charts to
//    dist/assets/chunithm/chart/, then serve it: `npx vite preview --port 4173`.
// 2. Install the browser driver once: `npm install --no-save puppeteer-core@24`
//    (run from this repository root; node_modules/ is ignored).
// 3. node scripts/bench-marisa.mjs <chart-dir> <new-output-dir> [runs=3]
//
// Environment: MARISA_URL (default http://127.0.0.1:4173), CHROMIUM (browser path,
// default /usr/bin/chromium), CHUNI_BINARY (default target/release/chuni-chart-rs).
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';

const require = createRequire(path.join(process.cwd(), 'noop.js'));
const puppeteer = (await import(require.resolve('puppeteer-core'))).default;

const [chartDir, outDir, runsArg = '3'] = process.argv.slice(2);
if (!chartDir || !outDir) throw Error('Usage: node scripts/bench-marisa.mjs <chart-dir> <new-output-dir> [runs]');
if (existsSync(outDir)) throw Error(`Output directory already exists: ${outDir}`);
mkdirSync(outDir, { recursive: true });

const RUNS = Number(runsArg);
const BASE = process.env.MARISA_URL || 'http://127.0.0.1:4173';
const binary = process.env.CHUNI_BINARY
  || path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../target/release/chuni-chart-rs');
const charts = readdirSync(chartDir).filter(f => f.endsWith('.c2s')).map(f => f.slice(0, -4)).sort();
if (!charts.length) throw Error(`No .c2s files in ${chartDir}`);

const median = a => { const s = [...a].sort((x, y) => x - y); const m = s.length >> 1; return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2; };

const browser = await puppeteer.launch({
  executablePath: process.env.CHROMIUM || '/usr/bin/chromium',
  headless: true,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});

// Same flow as MarisaBot WebApi.RenderUrl: persistent browser, one new page per image,
// 1x1 viewport, full-page JPEG (quality 90).
// waitIdle=false: stop as soon as the chart DOM and fonts are ready (pure generation time).
// waitIdle=true:  MarisaBot's actual networkidle0 wait.
async function marisa(name, waitIdle) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1, height: 1 });
  const url = `${BASE}/chunithm/preview?name=${encodeURIComponent(name)}`;
  const t0 = performance.now();
  if (waitIdle) {
    await page.goto(url, { waitUntil: ['networkidle0', 'networkidle2', 'load', 'domcontentloaded'], timeout: 0 });
  } else {
    await page.goto(url, { waitUntil: 'load', timeout: 0 });
    await page.waitForFunction(() => document.querySelector('#app > div')?.childElementCount > 0, { timeout: 0, polling: 'mutation' });
    await page.evaluate(async () => {
      await document.fonts.ready;
      await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    });
  }
  const t1 = performance.now();
  await page.screenshot({ path: path.join(outDir, `${name}.marisa.jpg`), fullPage: true, type: 'jpeg', quality: 90 });
  const t2 = performance.now();
  await page.close();
  return { html: t1 - t0, shot: t2 - t1, total: t2 - t0 };
}

// One complete CLI render: process start, parse, draw and encode.
function rust(name, format) {
  const t0 = performance.now();
  execFileSync(binary, ['render', path.join(chartDir, `${name}.c2s`), '-o', path.join(outDir, `${name}.rs.${format}`)], { stdio: 'ignore' });
  return performance.now() - t0;
}

await marisa(charts[0], false);
rust(charts[0], 'png');

const rows = [];
for (const name of charts) {
  const m = [], idle = [], png = [], jpg = [];
  for (let i = 0; i < RUNS; i++) {
    m.push(await marisa(name, false));
    idle.push((await marisa(name, true)).total);
    png.push(rust(name, 'png'));
    jpg.push(rust(name, 'jpg'));
  }
  const row = {
    name,
    html: median(m.map(x => x.html)),
    shot: median(m.map(x => x.shot)),
    total: median(m.map(x => x.total)),
    idleTotal: median(idle),
    rustPng: median(png),
    rustJpg: median(jpg),
  };
  rows.push(row);
  console.log(JSON.stringify(row));
}
await browser.close();

const fmt = key => {
  const values = rows.map(r => r[key]);
  return `${Math.round(median(values))} / ${Math.round(Math.max(...values))}`;
};
console.log(`\n${rows.length} charts, ${RUNS} runs each; median / slowest (ms)`);
console.log(`Marisa HTML generation     ${fmt('html')}`);
console.log(`Marisa browser screenshot  ${fmt('shot')}`);
console.log(`Marisa total               ${fmt('total')}`);
console.log(`Marisa total (networkidle) ${fmt('idleTotal')}`);
console.log(`chuni-chart-rs PNG         ${fmt('rustPng')}`);
console.log(`chuni-chart-rs JPEG        ${fmt('rustJpg')}`);
