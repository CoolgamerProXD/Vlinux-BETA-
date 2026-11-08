// Relative performance measurement of a built frontend (`node scripts/perf.mjs <dist-dir> <label>`).
//
// WHAT THIS IS: headless Chromium on a Linux sandbox with SOFTWARE rendering and the browser-preview
// back end. The numbers compare one build with another on the same machine in the same run. They
// are NOT frame rates or CPU use on a Windows laptop, and must not be quoted as such.
//
// Measured, per build, with all six apps open (Distributions, Linux Setup, Terminal, Files,
// Settings, System): DOM node count, JS heap after a forced GC, main-thread busy time while idle,
// and the frame-to-frame time while a window is dragged (with backdrop blur on and, where the build
// has the option, off). Each figure is the median of REPEATS runs.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { launch } from './browser.mjs';

const dist = path.resolve(process.argv[2] ?? 'dist');
const label = process.argv[3] ?? dist;
const REPEATS = Number(process.env.REPEATS ?? 3);
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
const server = http.createServer((req, res) => {
  let p = path.join(dist, decodeURIComponent(new URL(req.url, 'http://x').pathname));
  if (!p.startsWith(dist)) { res.writeHead(403).end(); return; }
  if (!fs.existsSync(p) || fs.statSync(p).isDirectory()) p = path.join(dist, 'index.html');
  res.writeHead(200, { 'content-type': mime[path.extname(p)] ?? 'application/octet-stream' });
  fs.createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/?wsl=ready`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const median = (a) => { const s = [...a].sort((x, y) => x - y); return s.length ? s[Math.floor(s.length / 2)] : NaN; };
const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return s.length ? s[Math.min(s.length - 1, Math.floor(s.length * p))] : NaN; };

const browser = await launch();

async function once(blur) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1600, height: 900 });
  await page.goto(url, { waitUntil: 'networkidle0' });
  await page.evaluate((b) => {
    localStorage.clear();
    const c = { runtime: { selected_distro: 'archlinux' } };
    if (b === false) c.appearance = { blur: false };
    localStorage.setItem('vlinux.config', JSON.stringify(c));
  }, blur);
  await page.reload({ waitUntil: 'networkidle0' });
  await page.waitForSelector('.shell.ready');
  await sleep(900);
  for (const name of ['Distributions', 'Linux Setup', 'Terminal', 'Files', 'Settings', 'System']) {
    const sel = `.dock .item[aria-label^="${name}"]`;
    if (await page.$(sel)) { await page.click(sel); await sleep(450); }
  }
  await sleep(1200);
  const cdp = await page.createCDPSession();
  await cdp.send('Performance.enable');
  await cdp.send('HeapProfiler.enable');
  await cdp.send('HeapProfiler.collectGarbage');
  const m = async (n) => (await cdp.send('Performance.getMetrics')).metrics.find((x) => x.name === n)?.value ?? 0;
  const nodes = await page.evaluate(() => document.getElementsByTagName('*').length);
  const wins = await page.$$eval('section.win', (w) => w.length);
  const heapMiB = (await m('JSHeapUsedSize')) / 1048576;
  const listeners = await m('JSEventListeners');

  const t0 = await m('TaskDuration'); const w0 = Date.now();
  await sleep(5000);
  const idlePct = (((await m('TaskDuration')) - t0) / ((Date.now() - w0) / 1000)) * 100;

  // Drag the front window and record the time between animation frames.
  await page.evaluate(() => {
    window.__d = [];
    let last = performance.now();
    const tick = (t) => { window.__d.push(t - last); last = t; if (window.__run) requestAnimationFrame(tick); };
    window.__run = true;
    requestAnimationFrame(tick);
  });
  const hb = await page.$eval('.win:not(.hidden):last-of-type header, .win.focused header', (e) => e.getBoundingClientRect().toJSON());
  await page.mouse.move(hb.x + 140, hb.y + 16);
  await page.mouse.down();
  for (let i = 0; i < 90; i++) { await page.mouse.move(hb.x + 140 + (i % 45) * 8, hb.y + 16 + (i % 30) * 4); await sleep(8); }
  await page.mouse.up();
  const deltas = await page.evaluate(() => { window.__run = false; return window.__d.slice(2); });
  await page.close();
  return { wins, nodes, heapMiB, listeners, idlePct, dragMedianMs: median(deltas), dragP95Ms: pct(deltas, 0.95), frames: deltas.length };
}

const report = { label, repeats: REPEATS, note: 'Linux headless Chromium, software rendering, preview back end: relative only', modes: {} };
const modes = process.env.MODES ? process.env.MODES.split(',') : ['default', 'noblur'];
for (const mode of modes) {
  const runs = [];
  for (let i = 0; i < REPEATS; i++) runs.push(await once(mode === 'noblur' ? false : undefined));
  const med = (k) => Number(median(runs.map((r) => r[k])).toFixed(2));
  report.modes[mode] = {
    windowsOpen: med('wins'), domNodes: med('nodes'), jsHeapMiB: med('heapMiB'), eventListeners: med('listeners'),
    idleMainThreadBusyPct: med('idlePct'), dragFrameMedianMs: med('dragMedianMs'), dragFrameP95Ms: med('dragP95Ms'), dragFrames: med('frames'),
  };
}
let size = 0; let js = 0; let css = 0;
const walk = (d) => { for (const f of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, f.name); if (f.isDirectory()) walk(p); else { const n = fs.statSync(p).size; size += n; if (f.name.endsWith('.js') && f.name.startsWith('index')) js += n; if (f.name.endsWith('.css')) css += n; } } };
walk(dist);
report.bundle = { distBytes: size, indexJsBytes: js, cssBytes: css };
console.log(JSON.stringify(report, null, 2));
await browser.close();
server.close();
