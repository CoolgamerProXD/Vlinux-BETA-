// Drives the REAL VLINUX window (Windows WebView2) over the Chrome DevTools Protocol.
//
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
//   .\vlinux.exe            # then:  node scripts/webview-smoke.mjs
//
// What it proves (and what it does not): the production bundle renders in WebView2 under the shipped
// CSP, the desktop appears (not a blank window), real Tauri IPC reaches Rust, Distributions shows the
// REAL state of the machine it runs on (never the simulated preview), and a few shell shortcuts work.
// It never starts a distribution and writes nothing but its own report/screenshot into OUT_DIR.
import fs from 'node:fs';
import path from 'node:path';
import puppeteer from 'puppeteer-core';

const port = process.env.CDP_PORT ?? '9222';
const out = process.env.OUT_DIR ?? 'artifacts';
fs.mkdirSync(out, { recursive: true });

const results = [];
const check = (name, ok, detail = '') => {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  -  ' + detail : ''}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const t0 = Date.now();
let browser;
for (let i = 0; i < 60 && !browser; i++) {
  try {
    browser = await puppeteer.connect({ browserURL: `http://127.0.0.1:${port}`, defaultViewport: null });
  } catch {
    await sleep(1000);
  }
}
if (!browser) {
  console.log('FAIL  could not attach to WebView2 over CDP');
  for (const ep of ['/json/version', '/json/list']) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}${ep}`);
      console.log(`GET ${ep} -> ${r.status} ${(await r.text()).slice(0, 600)}`);
    } catch (e) {
      console.log(`GET ${ep} failed: ${e}`);
    }
  }
  process.exit(2);
}
const connectMs = Date.now() - t0;

let page;
for (let i = 0; i < 30 && !page; i++) {
  page = (await browser.pages()).find((p) => /^https?:\/\/(tauri\.localhost|localhost|127\.0\.0\.1)/.test(p.url()) || p.url().startsWith('tauri://'));
  if (!page) await sleep(500);
}
if (!page) {
  console.log('FAIL  no VLINUX page found; pages: ' + (await browser.pages()).map((p) => p.url()).join(', '));
  process.exit(2);
}

const problems = [];
page.on('pageerror', (e) => problems.push('pageerror: ' + String(e)));
page.on('console', (m) => {
  if (m.type() === 'error') problems.push('console.error: ' + m.text());
});
page.on('requestfailed', (r) => problems.push('requestfailed: ' + r.url() + ' ' + (r.failure()?.errorText ?? '')));

// Reload with the listeners attached so start-up errors and CSP violations are seen.
// Time from process start to the desktop being ready, on one clock (the CI step passes the start time).
const mark = await page.evaluate(() => {
  const m = performance.getEntriesByName('vlinux-ready')[0];
  return m ? { epoch: performance.timeOrigin + m.startTime, sinceNav: m.startTime } : null;
});
let launchToReadyMs = null;
if (mark && process.env.PROC_START_MS) launchToReadyMs = Math.round(mark.epoch - Number(process.env.PROC_START_MS));
if (mark) console.log(`::notice title=Launch to desktop (real WebView2, runner)::${launchToReadyMs !== null ? launchToReadyMs + ' ms from process start to the desktop being ready; ' : ''}${Math.round(mark.sinceNav)} ms from page navigation start`);
const r0 = Date.now();
await page.reload({ waitUntil: 'load' });
await page.waitForSelector('.shell.ready', { timeout: 30000 }).catch(() => {});
const readyMs = Date.now() - r0;
const ready = !!(await page.$('.shell.ready'));
check('desktop renders in WebView2 (shell is ready, not a blank window)', ready, `${readyMs} ms from reload to ready`);
console.log(`::notice title=WebView2 render::CDP attach ${connectMs} ms after launch; reload to desktop ready ${readyMs} ms`);
await sleep(1200);

const ua = await page.evaluate(() => navigator.userAgent);
console.log('user agent: ' + ua);
check('running inside Tauri (IPC bridge present)', await page.evaluate(() => typeof window.__TAURI_INTERNALS__ === 'object'));
check('the dock, top bar and wallpaper are drawn', (await page.$$('.dock button, .dock a')).length >= 5 && !!(await page.$('.topbar, header')));
const box = await page.evaluate(() => ({ w: innerWidth, h: innerHeight }));
check('window has a usable size', box.w >= 800 && box.h >= 560, `${box.w}x${box.h}`);
const styled = await page.evaluate(() => {
  const el = document.querySelector('.glass');
  return !!el && getComputedStyle(el).backdropFilter !== undefined && getComputedStyle(document.body).backgroundColor !== 'rgba(0, 0, 0, 0)';
});
check('stylesheets and glass panels are applied under the shipped CSP', styled);

const welcome = await page.$('[data-testid="welcome"]');
check('first launch shows the welcome guidance (fresh profile)', !!welcome);
await page.screenshot({ path: path.join(out, 'webview-welcome.png') });

// Real IPC: the overview comes from Rust. Never started/changed anything.
const ipc = await page.evaluate(async () => {
  try {
    const o = await window.__TAURI_INTERNALS__.invoke('distro_overview', { force: false });
    return { ok: true, status: o?.runtime?.status ?? null, distros: Array.isArray(o?.distros) ? o.distros.length : null };
  } catch (e) {
    return { ok: false, error: String(e?.message ?? e) };
  }
});
check('real IPC: distro_overview answers from Rust', ipc.ok, ipc.ok ? `status=${ipc.status}, distributions=${ipc.distros}` : ipc.error);

// UI -> Distributions with REAL data
if (welcome) await page.click('[data-testid="welcome-detect"]');
else await page.keyboard.down('Alt'), await page.keyboard.press('F1'), await page.keyboard.up('Alt');
await sleep(1000);
const settled = await page
  .waitForFunction(() => {
    const e = document.querySelector('[data-testid="distros-empty"]');
    return !e || e.getAttribute('data-kind') !== 'loading';
  }, { timeout: 25000 })
  .then(() => true)
  .catch(() => false);
check('Distributions leaves the loading state', settled);
const dtext = await page.evaluate(() => document.body.innerText);
check('Distributions shows REAL data, not the simulated preview', !dtext.includes('SIMULATED: this is the browser preview') && !(await page.$('[data-testid="distros-sim-banner"]')));
const emptyTitle = await page.evaluate(() => document.querySelector('[data-testid="distros-empty"] h3')?.textContent ?? null);
console.log(`::notice title=Real Distributions view::${emptyTitle ?? 'distribution cards shown'} (runner state; no distribution is created or started by this test)`);
await page.screenshot({ path: path.join(out, 'webview-distributions.png') });

// Keyboard: launcher opens and closes
await page.keyboard.down('Alt'); await page.keyboard.press('F1'); await page.keyboard.up('Alt');
await sleep(600);
check('Alt+F1 opens the launcher', !!(await page.$('[role="dialog"][aria-label="Application launcher"]')));
await page.keyboard.press('Escape');
await sleep(500);
check('Esc closes the launcher', !(await page.$('[role="dialog"][aria-label="Application launcher"]')));

// Open each remaining app from the dock and make sure nothing throws.
const dockLabels = await page.$$eval('.dock button', (bs) => bs.map((b) => b.getAttribute('aria-label') ?? b.title ?? ''));
for (let i = 0; i < dockLabels.length; i++) {
  const label = dockLabels[i];
  if (!label || /launcher|applications/i.test(label) || /terminal/i.test(label)) continue;
  const btns = await page.$$('.dock button');
  if (!btns[i]) continue;
  await btns[i].click();
  await sleep(700);
}
check('apps open from the dock without errors', problems.length === 0, problems.slice(0, 3).join(' | '));
await page.screenshot({ path: path.join(out, 'webview-apps.png') });

check('no console errors, CSP violations or failed requests during the whole run', problems.length === 0, problems.slice(0, 5).join(' | '));

const failed = results.filter((r) => !r.ok).length;
fs.writeFileSync(path.join(out, 'webview-smoke.json'), JSON.stringify({ results, problems, connectMs, readyMs, launchToReadyMs, userAgent: ua }, null, 2));
console.log(`::notice title=WebView2 smoke (real window)::${results.length - failed}/${results.length} checks passed${failed ? ', FAILED: ' + results.filter((r) => !r.ok).map((r) => r.name).join('; ') : ''}`);
await browser.disconnect();
process.exit(failed ? 1 : 0);
