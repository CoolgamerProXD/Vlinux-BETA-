// Desktop integration checks against the REAL VLINUX window on Windows (run by CI after the
// WebView2 smoke, with the same CDP-enabled build running):
//   full screen, F11, Ctrl+Alt+Q (return to Windows), resize, launcher, workspaces, power menu and
//   closing/reopening windows.
//
// Keys are sent as REAL operating-system keystrokes (scripts/win-input.ps1 -> keybd_event) to the
// foreground window, so they pass through Windows and WebView2 the way a user's keyboard does. When
// Windows will not give the window the foreground (a CI session quirk), the same checks fall back to
// keys injected over CDP and are labelled "[CDP key]" - those prove the app's key handling, not
// Windows' delivery. State is read back from the app itself (is_fullscreen, document visibility,
// layout) and from Windows (IsIconic).
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import puppeteer from 'puppeteer-core';

process.on('uncaughtException', (e) => { console.log('FAIL  desktop smoke crashed: ' + (e?.stack ?? e)); process.exit(3); });
process.on('unhandledRejection', (e) => { console.log('FAIL  desktop smoke crashed: ' + (e?.stack ?? e)); process.exit(3); });

const port = process.env.CDP_PORT ?? '9222';
const out = process.env.OUT_DIR ?? 'artifacts';
const proc = process.env.VLX_PROC ?? 'vlinux-cdp';
fs.mkdirSync(out, { recursive: true });
const helper = path.join(path.dirname(fileURLToPath(import.meta.url)), 'win-input.ps1');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const results = [];
const check = (name, ok, detail = '') => {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  -  ' + detail : ''}`);
};
const skipNote = (name, why) => {
  results.push({ name, ok: true, skipped: true, detail: why });
  console.log(`SKIP  ${name}  -  ${why}`);
};
const ps = (action, arg = '') =>
  execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', helper, '-Action', action, '-Arg', arg, '-Proc', proc], { encoding: 'utf8', timeout: 30000 }).trim();

let browser;
for (let i = 0; i < 30 && !browser; i++) {
  try { browser = await puppeteer.connect({ browserURL: `http://127.0.0.1:${port}`, defaultViewport: null }); } catch { await sleep(1000); }
}
if (!browser) { console.log('FAIL  could not attach to WebView2 over CDP'); process.exit(2); }
const page = (await browser.pages()).find((p) => /^https?:\/\/(tauri\.localhost|localhost|127\.0\.0\.1)/.test(p.url()) || p.url().startsWith('tauri://'));
if (!page) { console.log('FAIL  no VLINUX page'); process.exit(2); }

const problems = [];
page.on('pageerror', (e) => problems.push(String(e)));
await page.reload({ waitUntil: 'load' });
await page.waitForSelector('.shell.ready', { timeout: 30000 });
await sleep(1500);

const state = () => page.evaluate(async () => ({
  fs: await window.__TAURI_INTERNALS__.invoke('is_fullscreen'),
  vis: document.visibilityState,
  focus: document.hasFocus(),
  w: innerWidth, h: innerHeight,
  launcher: !!document.querySelector('[role="dialog"][aria-label="Application launcher"]'),
  power: !!document.querySelector('[role="dialog"][aria-label="Power and exit"]'),
  ws: document.querySelector('nav.ws [aria-current="true"]')?.textContent?.trim() ?? null,
  wins: document.querySelectorAll('section.win:not(.hidden)').length,
}));
const until = async (pred, ms = 5000) => {
  const t0 = Date.now();
  let s = await state();
  while (!pred(s) && Date.now() - t0 < ms) { await sleep(200); s = await state(); }
  return s;
};

// Which kind of keys can we use? A canary decides: type three letters with REAL OS keystrokes into a
// text box in the page. If they arrive, shortcuts are tested with real keystrokes; if Windows (or this
// CI session) drops injected input, shortcuts are tested with CDP-injected keys and labelled so.
let os = false;
let info = '';
try {
  ps('focus');
  await sleep(300);
  info = ps('info');
  await page.evaluate(() => { const i = document.createElement('input'); i.id = 'vlx-canary'; i.style.cssText = 'position:fixed;left:0;top:0;z-index:99999'; document.body.appendChild(i); i.focus(); });
  ps('keys', 'V L X');
  await sleep(300);
  const typed = await page.evaluate(() => document.getElementById('vlx-canary')?.value ?? '');
  os = typed === 'vlx';
  console.log(`canary: typed ${JSON.stringify(typed)} (expected "vlx"); ${info}`);
} catch (e) { console.log('focus/canary helper failed: ' + String(e).slice(0, 300)); }
// Diagnose WHERE real keystrokes go: log every keydown the page receives, then send a harmless key
// (F7 - no handler) with nothing focused, and again with a text box focused.
await page.evaluate(() => { window.__keys = []; window.addEventListener('keydown', (e) => window.__keys.push(`${e.ctrlKey ? 'Ctrl+' : ''}${e.altKey ? 'Alt+' : ''}${e.shiftKey ? 'Shift+' : ''}${e.code || 'key:' + e.key}${e.defaultPrevented ? '(handled)' : ''}`), true); });
const keysSeen = () => page.evaluate(() => [...window.__keys]);
let delivered = { nothingFocused: null, textBoxFocused: null };
if (os) {
  await page.evaluate(() => { const i = document.getElementById('vlx-canary'); i?.blur(); document.body.focus?.(); });
  await page.evaluate(() => { window.__keys.length = 0; });
  ps('keys', 'F7'); await sleep(300);
  delivered.nothingFocused = (await keysSeen()).some((k) => k.includes('F7'));
  await page.evaluate(() => { window.__keys.length = 0; document.getElementById('vlx-canary')?.focus(); });
  ps('keys', 'F7'); await sleep(300);
  delivered.textBoxFocused = (await keysSeen()).some((k) => k.includes('F7'));
  console.log(`delivery of a plain F7 key: nothing focused=${delivered.nothingFocused}, text box focused=${delivered.textBoxFocused}`);
  await page.evaluate(() => { window.__keys.length = 0; });
}
await page.evaluate(() => document.getElementById('vlx-canary')?.remove());
await page.evaluate(() => (document.activeElement instanceof HTMLElement) && document.activeElement.blur());
const kind = os ? '[OS key]' : '[CDP key]';
console.log(`::notice title=Desktop smoke input::${os ? 'real operating-system keystrokes (keybd_event with scan codes) reach the page' : 'real OS keystrokes did NOT reach the page in this CI session (' + info + '); shortcuts are tested with keys injected over CDP, which proves the app\'s key handling but not Windows\' delivery'}`);

const CDP = { 'F11': ['F11'], 'ESC': ['Escape'], 'ALTF1': ['Alt', 'F1'], 'ALT2': ['Alt', 'Digit2'], 'ALT1': ['Alt', 'Digit1'], 'ALTW': ['Alt', 'KeyW'], 'POWER': ['Alt', 'Shift', 'KeyQ'], 'CAQ': ['Control', 'Alt', 'KeyQ'] };
const SEND = { 'F11': 'F11', 'ESC': 'Esc', 'ALTF1': 'Alt+F1', 'ALT2': 'Alt+2', 'ALT1': 'Alt+1', 'ALTW': 'Alt+W', 'POWER': 'Alt+Shift+Q', 'CAQ': 'Ctrl+Alt+Q' };
async function press(id) {
  if (os) { await page.evaluate(() => { window.__keys.length = 0; }); ps('keys', SEND[id]); await sleep(150); console.log(`  sent ${id}: page saw [${(await keysSeen()).join(', ')}]`); return; }
  const keys = CDP[id];
  for (const k of keys) await page.keyboard.down(k);
  for (const k of [...keys].reverse()) await page.keyboard.up(k);
}

const s0 = await state();
check('the shell starts in full screen (default setting)', s0.fs === true, `fullscreen=${s0.fs}, ${s0.w}x${s0.h}`);

await press('F11');
let s = await until((x) => x.fs === false);
check(`${kind} F11 leaves full screen`, s.fs === false, `fullscreen=${s.fs}`);

// resize the real window
try {
  ps('move', '40,30,920,640');
  s = await until((x) => x.w < 1000 && x.w >= 700);
  check('the real window can be resized and the page follows', s.w < 1000 && s.w >= 700, `${s.w}x${s.h}`);
  const lay = await page.evaluate(() => {
    const vis = (sel) => { const r = document.querySelector(sel)?.getBoundingClientRect(); return !!r && r.width > 0 && r.left >= -1 && r.top >= -1 && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1; };
    return { dock: vis('.dock'), bar: vis('header.bar') };
  });
  check('after resizing, the dock and top bar are fully inside the window', lay.dock && lay.bar, JSON.stringify(lay));
  await page.screenshot({ path: path.join(out, 'desktop-resized.png') });
} catch (e) { check('window resize helper ran', false, String(e).slice(0, 200)); }

const beforeEnter = (await state()).fs;
await press('F11');
s = await until((x) => x.fs === true);
check(`${kind} F11 enters full screen`, beforeEnter === false && s.fs === true, `before=${beforeEnter}, after=${s.fs}`);

await press('ALTF1');
s = await until((x) => x.launcher);
const lOpen = s.launcher;
check(`${kind} Alt+F1 opens the launcher`, s.launcher);
await press('ESC');
s = await until((x) => !x.launcher);
check(`${kind} Esc closes the launcher`, lOpen && !s.launcher);

await press('ALT2');
s = await until((x) => x.ws === '2');
check(`${kind} Alt+2 switches to workspace 2`, s.ws === '2', `workspace ${s.ws}`);
await press('ALT1');
s = await until((x) => x.ws === '1');
check(`${kind} Alt+1 switches back to workspace 1`, s.ws === '1', `workspace ${s.ws}`);

await press('POWER');
s = await until((x) => x.power);
const pOpen = s.power;
check(`${kind} Alt+Shift+Q opens the power menu`, s.power);
await press('ESC');
s = await until((x) => !x.power);
check(`${kind} Esc closes the power menu`, pOpen && !s.power);

// window close and reopen (Settings: needs no distribution)
const openSettings = async () => {
  const labels = await page.$$eval('.dock button', (bs) => bs.map((b) => b.getAttribute('aria-label') ?? ''));
  const i = labels.findIndex((l) => /^Settings/.test(l));
  if (i < 0) return false;
  await (await page.$$('.dock button'))[i].click();
  return true;
};
const before = (await state()).wins;
await openSettings();
s = await until((x) => x.wins === before + 1);
check('a window opens from the dock', s.wins === before + 1, `${s.wins} visible`);
await press('ALTW');
s = await until((x) => x.wins === before);
const closedOk = s.wins === before;
check(`${kind} Alt+W closes the focused window`, closedOk, `${s.wins} visible`);
await openSettings();
s = await until((x) => x.wins === before + 1);
check('the same window opens again after being closed', closedOk && s.wins === before + 1);
await press('ALTW');
await until((x) => x.wins === before);

// return to Windows
await press('CAQ');
s = await until((x) => x.vis === 'hidden' && x.fs === false, 6000);
const iconic = ps('iconic');
check(`${kind} Ctrl+Alt+Q returns to Windows (leaves full screen and minimises)`, s.fs === false && (iconic === 'True' || s.vis === 'hidden'), `fullscreen=${s.fs}, visibility=${s.vis}, minimised=${iconic}`);

ps('restore');
s = await until((x) => x.vis === 'visible', 6000);
const stillThere = await page.evaluate(() => !!document.querySelector('.shell.ready .dock') && !!document.querySelector('header.bar'));
check('restoring from the minimised state brings the desktop back unchanged', s.vis === 'visible' && stillThere, `visibility=${s.vis}`);
await page.screenshot({ path: path.join(out, 'desktop-restored.png') });

// full screen again from the top-bar button (DOM click -> real set_fullscreen)
await page.evaluate(() => document.querySelector('button[aria-label="Enter full screen"]')?.click());
s = await until((x) => x.fs === true);
check('the top-bar full-screen button enters full screen', s.fs === true);

check('no script errors during the run', problems.length === 0, problems.slice(0, 3).join(' | '));

const failed = results.filter((r) => !r.ok).length;
fs.writeFileSync(path.join(out, 'desktop-smoke.json'), JSON.stringify({ input: os ? 'os' : 'cdp', results }, null, 2));
console.log(`::notice title=Desktop smoke (real window, ${os ? 'real OS keystrokes' : 'CDP keys'})::${results.length - failed}/${results.length} checks passed${failed ? ', FAILED: ' + results.filter((r) => !r.ok).map((r) => r.name).join('; ') : ''}`);
await browser.disconnect();
process.exit(failed ? 1 : 0);
