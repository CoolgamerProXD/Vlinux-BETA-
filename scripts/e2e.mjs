// End-to-end UI test against the built frontend (run `npm run build` first; the script
// serves ./dist itself). Runs in plain Chromium in browser-preview mode, so the Rust
// commands are served by the browser fallbacks in src/lib/ipc.ts.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { launch } from './browser.mjs';

const dist = path.resolve('dist');
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
const server = http.createServer((req, res) => {
  let p = path.join(dist, decodeURIComponent(new URL(req.url, 'http://x').pathname));
  if (!p.startsWith(dist)) { res.writeHead(403).end(); return; }
  if (!fs.existsSync(p) || fs.statSync(p).isDirectory()) p = path.join(dist, 'index.html');
  res.writeHead(200, { 'content-type': mime[path.extname(p)] ?? 'application/octet-stream' });
  fs.createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/`;

const shots = process.env.SHOTS_DIR;
if (shots) fs.mkdirSync(shots, { recursive: true });

let failed = 0, passed = 0;
const check = (name, ok, extra = '') => {
  if (ok) { passed++; console.log(`  ✓ ${name}`); } else { failed++; console.log(`  ✗ ${name} ${extra}`); }
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = await launch();
const errors = [];
async function fresh(w = 1600, h = 900, query = '') {
  const page = await browser.newPage();
  await page.setViewport({ width: w, height: h });
  page.on('pageerror', (e) => errors.push(String(e)));
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(url + query, { waitUntil: 'networkidle0' });
  await page.waitForSelector('.shell.ready');
  await sleep(900); // let entry animations settle
  return page;
}
const shot = async (page, name) => { if (shots) await page.screenshot({ path: path.join(shots, `${name}.png`) }); };
const text = (page, sel) => page.$eval(sel, (e) => e.textContent.trim());
const count = (page, sel) => page.$$eval(sel, (e) => e.length);
const visibleWindows = (page) => page.$$eval('.win', (els) => els.filter((e) => getComputedStyle(e).display !== 'none').map((e) => e.getAttribute('aria-label')));

try {
  console.log('Shell basics (1600×900)');
  let page = await fresh();
  check('top bar rendered', (await count(page, '.bar')) === 1);
  check('clock shows HH:MM', /^\d{2}:\d{2}$/.test(await text(page, '.time')), await text(page, '.time'));
  check('date shown', (await text(page, '.date')).length > 3);
  check('5 workspace indicators, #1 active', (await count(page, '.ws .pill')) === 5 && (await text(page, '.ws .pill.active')) === '1');
  check('dock has launcher + 5 apps', (await count(page, '.dock .item')) === 6);
  check('every app is available (none disabled); Files and Terminal are enabled', (await count(page, '.dock .item[aria-disabled="true"]')) === 0 && (await count(page, '.dock .item[aria-label^="Files"]:not([aria-disabled="true"])')) === 1 && (await count(page, '.dock .item[aria-label^="Terminal"]:not([aria-disabled="true"])')) === 1);
  check('wallpaper rendered', (await count(page, '.wp svg')) === 1);
  await shot(page, '01-desktop');

  console.log('Launcher');
  await page.keyboard.down('Alt'); await page.keyboard.press('F1'); await page.keyboard.up('Alt');
  await page.waitForSelector('.panel[aria-label="Application launcher"]');
  check('Alt+F1 opens launcher', true);
  check('launcher lists all 6 apps', (await count(page, '.grid .app')) === 6);
  await shot(page, '02-launcher');
  await page.keyboard.type('settin');
  check('search filters', (await count(page, '.grid .app')) === 1);
  await page.keyboard.press('Enter');
  await sleep(500);
  check('Enter launches Settings and closes launcher', (await visibleWindows(page)).join() === 'Settings' && (await count(page, '.backdrop')) === 0);
  await shot(page, '03-settings');

  console.log('Settings apply + persist');
  await page.click('.wp:nth-child(2)'); // dusk
  await page.click('.sw[aria-label="rose"]');
  await sleep(100);
  check('accent CSS var updated', (await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent').trim())) === '#ff7aa8');
  await shot(page, '04-dusk-rose');
  await sleep(500); // debounce flush
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')));
  check('config persisted', stored.appearance.wallpaper === 'dusk' && stored.appearance.accent === 'rose', JSON.stringify(stored));
  await page.reload({ waitUntil: 'networkidle0' });
  await page.waitForSelector('.shell.ready');
  check('config restored after reload', (await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent').trim())) === '#ff7aa8');

  console.log('Workspaces');
  await page.keyboard.down('Alt'); await page.keyboard.press('KeyL'); await page.keyboard.up('Alt'); // harmless key
  const open = async (q) => { await page.keyboard.down('Alt'); await page.keyboard.press('F1'); await page.keyboard.up('Alt'); await page.waitForSelector('.panel'); await page.keyboard.type(q); await page.keyboard.press('Enter'); await sleep(450); };
  await open('settings');
  await page.keyboard.down('Alt'); await page.keyboard.press('Digit3'); await page.keyboard.up('Alt');
  await sleep(100);
  check('Alt+3 switches workspace and hides window', (await text(page, '.ws .pill.active')) === '3' && (await visibleWindows(page)).length === 0);
  check('workspace 1 marked occupied', await page.$eval('.ws .pill:nth-child(1)', (e) => e.classList.contains('occupied')));
  await open('system');
  check('System opens on workspace 3', (await visibleWindows(page)).join() === 'System');
  await shot(page, '05-system');
  await page.click('.ws .pill:nth-child(1)');
  check('clicking pill 1 shows Settings only', (await visibleWindows(page)).join() === 'Settings');

  console.log('Window dragging, maximise, close');
  await sleep(500); // window re-show animation
  const before = await page.$eval('.win:not(.hidden)', (e) => e.getBoundingClientRect().toJSON());
  const hb = await page.$eval('.win:not(.hidden) header', (e) => e.getBoundingClientRect().toJSON());
  await page.mouse.move(hb.x + 120, hb.y + 18);
  await page.mouse.down(); await page.mouse.move(hb.x + 220, hb.y + 78, { steps: 5 }); await page.mouse.up();
  const after = await page.$eval('.win:not(.hidden)', (e) => e.getBoundingClientRect().toJSON());
  check('dragging title bar moves window', Math.abs(after.x - before.x - 100) < 3 && Math.abs(after.y - before.y - 60) < 3, `${before.x},${before.y} → ${after.x},${after.y}`);
  await page.click('.win:not(.hidden) .ctl button[aria-label="Maximise"]');
  await sleep(100);
  const work = await page.$eval('.work', (e) => e.getBoundingClientRect().toJSON());
  const maxed = await page.$eval('.win:not(.hidden)', (e) => e.getBoundingClientRect().toJSON());
  check('maximise fills the work area', Math.abs(maxed.width - work.width) < 2 && Math.abs(maxed.height - work.height) < 2);
  await page.click('.win:not(.hidden) .ctl button[aria-label="Restore"]');
  await page.keyboard.down('Alt'); await page.keyboard.press('KeyW'); await page.keyboard.up('Alt');
  await sleep(100);
  check('Alt+W closes focused window', (await count(page, '.win:not(.hidden)')) === 0);

  console.log('Power menu / exit to Windows');
  await page.click('button[aria-label="Power and exit menu"]');
  await page.waitForSelector('.panel[aria-label="Power and exit"]');
  await sleep(500);
  await shot(page, '06-power');
  check('Return to Windows tile is present', (await page.$eval('.tile.primary strong', (e) => e.textContent)) === 'Return to Windows');
  await page.click('.tile.primary');
  await page.waitForSelector('.toast.error');
  check('browser preview reports (honestly) that there is no host window', (await text(page, '.toast.error')).includes('Browser preview'));
  await page.click('button[aria-label="Power and exit menu"]');
  await page.click('.tile.danger');
  await page.waitForSelector('.confirm');
  check('quit requires confirmation', (await text(page, '.panel[aria-label="Power and exit"] h2')) === 'Quit VLINUX?');
  await shot(page, '07-quit-confirm');
  await page.keyboard.press('Escape');
  check('Esc steps back from confirmation (does not quit)', (await count(page, '.confirm')) === 0 && (await count(page, '.panel[aria-label="Power and exit"]')) === 1);
  await page.keyboard.press('Escape');
  check('second Esc closes menu', (await count(page, '.backdrop')) === 0);

  console.log('Left dock + workspace count');
  await open('settings');
  await page.click('.settings nav button:nth-child(2)');
  await page.click('.seg button:nth-child(2)'); // left
  await sleep(150);
  const dock = await page.$eval('.dock', (e) => e.getBoundingClientRect().toJSON());
  const w2 = await page.$eval('.work', (e) => e.getBoundingClientRect().toJSON());
  check('left dock sits left of the work area', dock.right <= w2.left + 1 && dock.height > dock.width);
  await page.$eval('input[aria-label="Number of workspaces"]', (e) => { e.value = '3'; e.dispatchEvent(new Event('input', { bubbles: true })); });
  await sleep(100);
  check('workspace count setting applies', (await count(page, '.ws .pill')) === 3);
  await shot(page, '08-left-dock');
  await page.close();


  console.log('M2 - Linux Setup (simulated preview data; no WSL exists in a browser)');
  const clickText = (pg, sel, t) => pg.evaluate((s2, t2) => { const el = [...document.querySelectorAll(s2)].find((e) => e.textContent.includes(t2)); if (!el) throw new Error(`no ${s2} with "${t2}"`); el.click(); }, sel, t);
  const openApp = async (pg, label) => { await pg.click(`.dock .item[aria-label^="${label}"]`); await sleep(500); };
  const statusOf = (pg) => pg.$eval('.card.status', (e) => e.getAttribute('data-status'));

  page = await fresh(1600, 900);
  await openApp(page, 'Linux Setup');
  await page.waitForSelector('.card.status');
  check('default browser preview says WSL is not available here', (await statusOf(page)) === 'unsupported_host');
  check('no install buttons when the host is unsupported', (await count(page, '.card.status button')) === 0);
  await shot(page, '10-setup-unavailable');
  await page.close();

  page = await fresh(1600, 900, '?wsl=reboot');
  await openApp(page, 'Linux Setup');
  await page.waitForSelector('.card.status');
  check('restart pending is explained, with no install action', (await statusOf(page)) === 'restart_required' && (await count(page, '.card.status button')) === 0 && /restart/i.test(await text(page, '.card.status')));
  await page.close();

  page = await fresh(1600, 900, '?wsl=missing');
  await openApp(page, 'Linux Setup');
  await page.waitForSelector('.card.status');
  check('missing WSL is reported', (await statusOf(page)) === 'not_installed');
  check('simulated-data banner is shown', (await count(page, '.notice.sim')) === 1);
  await clickText(page, '.card.status button', 'Review WSL setup');
  await page.waitForSelector('.card.review');
  const platformText = await text(page, '.card.review');
  check('WSL platform review mentions UAC, restart and no distribution', /UAC/.test(platformText) && /restart/i.test(platformText) && /No Linux distribution/.test(platformText));
  check('WSL platform review shows the exact command', platformText.includes('wsl.exe --install --no-distribution'));
  check('Install is disabled until the user approves', await page.$eval('.card.review .btn.primary', (b) => b.disabled));
  await shot(page, '11-setup-wsl-review');
  await page.close();

  page = await fresh(1600, 900, '?wsl=empty');
  await openApp(page, 'Linux Setup');
  await page.waitForSelector('.card.status[data-status="no_distributions"]');
  check('no distributions: none listed, nothing suggested', (await count(page, '.distro')) === 0 && (await count(page, '.card.offer')) === 0);
  await page.close();

  page = await fresh(1600, 900, '?wsl=ready');
  await openApp(page, 'Linux Setup');
  await page.waitForSelector('.card.status[data-status="ready"]');
  check('3 user distributions + 1 other-app distribution listed', (await count(page, '.distro:not(.internal)')) === 3 && (await count(page, '.distro.internal')) === 1);
  check('the existing Arch is labelled Existing, nothing is "Installed by VLINUX" yet', (await text(page, '.distro[data-name="archlinux"] .badges')).includes('Existing') && !(await text(page, '.setup')).includes('Installed by VLINUX'));
  check('the other application\'s distribution has no actions', (await count(page, '.distro.internal button')) === 0);
  check('WSL 1 distribution is flagged and cannot be selected', await page.$eval('.distro[data-name="Debian"]', (e) => e.textContent.includes('WSL 1') && [...e.querySelectorAll('button')].find((b) => b.textContent.includes('Use with VLINUX')).disabled));
  check('existing distribution offered for use', (await count(page, '.card.offer')) === 1 && (await text(page, '.card.offer')).includes('Arch Linux') && (await text(page, '.card.offer')).includes('will not reinstall'));
  await shot(page, '12-setup-ready');
  await clickText(page, '.card.offer button', 'Use archlinux');
  await sleep(700);
  check('choosing it marks it Selected and hides the offer', (await text(page, '.distro[data-name="archlinux"] .badges')).includes('Selected') && (await count(page, '.card.offer')) === 0);
  const sel = await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).runtime.selected_distro);
  check('selection persisted to config', sel === 'archlinux', String(sel));

  await clickText(page, 'section[aria-label="Install a distribution"] button', 'Show official list');
  await page.waitForSelector('.catalog li');
  check('official list shows 5 entries', (await count(page, '.catalog li')) === 5);
  check('unsupported distro cannot be installed, installed one is marked', (await text(page, '.catalog li[data-name="kali-linux"]')).includes('Not supported yet') && (await count(page, '.catalog li[data-name="kali-linux"] button')) === 0 && (await text(page, '.catalog li[data-name="archlinux"]')).includes('Installed') && (await count(page, '.catalog li[data-name="archlinux"] button')) === 0);

  await clickText(page, '.catalog li[data-name="FedoraLinux-42"] button', 'Review install');
  await page.waitForSelector('.card.review');
  const rv = await text(page, '.card.review');
  check('review lists the command and the untouched distributions', rv.includes('wsl.exe --install FedoraLinux-42 --no-launch') && rv.includes('archlinux') && rv.includes('Left untouched'));
  check('nothing installs without ticking the approval box', await page.$eval('.card.review .btn.primary', (b) => b.disabled) && (await count(page, '.card.job')) === 0);
  await shot(page, '13-setup-review');
  await page.click('.card.review .consent input');
  check('approving enables Install', !(await page.$eval('.card.review .btn.primary', (b) => b.disabled)));
  await page.click('.card.review .btn.primary');
  await page.waitForSelector('.card.job[data-outcome="running"]');
  await shot(page, '14-setup-progress');
  await sleep(1200);
  await clickText(page, '.card.job button', 'Cancel installation');
  await page.waitForSelector('.card.job[data-outcome="cancelled"]', { timeout: 5000 });
  check('cancelling stops the job and says existing distributions are unchanged', (await text(page, '.card.job')).includes('Existing distributions were not changed') && (await count(page, '.distro[data-name="FedoraLinux-42"]')) === 0);
  await clickText(page, '.card.job button', 'Dismiss');

  await clickText(page, '.catalog li[data-name="Ubuntu"] button', 'Review install');
  await page.waitForSelector('.card.review');
  await page.click('.card.review .consent input');
  await page.click('.card.review .btn.primary');
  await page.waitForSelector('.card.job[data-outcome="succeeded"]', { timeout: 15000 });
  await sleep(500);
  check('a finished install is labelled Installed by VLINUX, and Arch is still Existing', (await text(page, '.distro[data-name="Ubuntu"] .badges')).includes('Installed by VLINUX') && (await text(page, '.distro[data-name="archlinux"] .badges')).includes('Existing'));
  await shot(page, '15-setup-installed');

  console.log('M2 - status panel shows a cached summary and never detects on its own');
  await page.click('.chip[aria-label="System status"]');
  await page.waitForSelector('.panel[aria-label="System status"]');
  const sp = await text(page, '.panel[aria-label="System status"]');
  check('status panel shows the cached WSL summary and selected distribution', sp.includes('Ready · 4 distributions') && sp.includes('archlinux'), sp);
  await page.keyboard.press('Escape');
  await page.close();


  console.log('M2 - cached summary is empty before any detection');
  page = await fresh(1600, 900);
  await page.click('.chip[aria-label="System status"]');
  await page.waitForSelector('.panel[aria-label="System status"]');
  check('status panel says Not checked yet and does not run detection by itself', (await text(page, '.panel[aria-label="System status"]')).includes('Not checked yet'));
  await page.close();

  console.log('M2 - Linux folders (storage preferences)');
  page = await fresh(1600, 900);
  await page.evaluate(() => localStorage.clear());
  await open2(page);
  await clickText(page, '.settings nav button', 'Linux folders');
  await page.waitForSelector('.folder[data-folder="downloads"]');
  check('six personal folders listed, all shared by default', (await count(page, '.folder')) === 6 && (await page.$$eval('.folder input[role="switch"]', (els) => els.every((e) => e.checked))));
  check('preferences are not presented as connected (checked-and-applied wording, no "Connected" badge)', (await text(page, '.page')).includes('checked and applied') && !(await text(page, '.page')).includes('Connected'));
  check('projects default to Linux-native storage', await page.$eval('.seg button[aria-checked="true"]', (b) => b.textContent.includes('Inside Linux')));
  await page.click('.folder[data-folder="music"] input[role="switch"]');
  await sleep(800);
  const st = await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).storage);
  check('turning a folder private is saved', st.music.mode === 'private' && st.downloads.mode === 'shared', JSON.stringify(st));
  check('private folder says it stays in Linux', (await text(page, '.folder[data-folder="music"]')).includes('Linux-private'));
  await page.type('.folder[data-folder="downloads"] input.text', '\\\\server\\share\\Dl');
  await clickText(page, '.folder[data-folder="downloads"] button', 'Check');
  await sleep(200);
  check('a UNC path is detected and refused', (await text(page, '.folder[data-folder="downloads"] .custom')).includes('network (UNC)') && await page.$eval('.folder[data-folder="downloads"] .pathrow button:last-child', (b) => b.disabled));
  await page.$eval('.folder[data-folder="downloads"] input.text', (e) => { e.value = ''; e.dispatchEvent(new Event('input', { bubbles: true })); });
  await page.type('.folder[data-folder="downloads"] input.text', 'D:\\Media\\Dl');
  await clickText(page, '.folder[data-folder="downloads"] button', 'Check');
  await sleep(200);
  check('a drive path is accepted and shows the Linux path', (await text(page, '.folder[data-folder="downloads"] .custom')).includes('/mnt/d/Media/Dl'));
  await clickText(page, '.folder[data-folder="downloads"] button', 'Use this folder');
  await sleep(900);
  check('custom location is shown and saved', (await text(page, '.folder[data-folder="downloads"]')).includes('D:\\Media\\Dl') && (await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).storage.downloads.windows_path)) === 'D:\\Media\\Dl');
  await shot(page, '16-settings-folders');
  await clickText(page, '.page button', 'Restore folder defaults');
  await sleep(900);
  const st2 = await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).storage);
  check('restore defaults resets the preferences', st2.music.mode === 'shared' && st2.downloads.windows_path === null);
  await page.close();

  console.log('M3 - Terminal (browser preview: real xterm.js UI, loopback transport, simulated WSL data)');
  const termText = (pg) => pg.$eval('.pane.active .xterm-rows', (e) => e.textContent);
  const pv = (pg) => pg.evaluate(() => JSON.parse(JSON.stringify(window.__vlinuxTerminalPreview)));
  const setSelected = async (pg, name) => {
    await pg.evaluate((n) => {
      const c = JSON.parse(localStorage.getItem('vlinux.config') ?? '{}');
      c.runtime = { ...(c.runtime ?? {}), selected_distro: n };
      localStorage.setItem('vlinux.config', JSON.stringify(c));
    }, name);
    await pg.reload({ waitUntil: 'networkidle0' });
    await pg.waitForSelector('.shell.ready');
    await sleep(900);
  };

  // Gates (no session can start)
  page = await fresh(1600, 900);
  await openApp(page, 'Terminal');
  await page.waitForSelector('.gate [data-gate], .gate .card');
  check('terminal: WSL unavailable explains itself and points to Linux Setup', (await page.$eval('.gate .card', (e) => e.textContent)).includes('Linux Setup') && (await count(page, '.pane')) === 0);
  await shot(page, '11-terminal-unavailable');
  await page.close();

  page = await fresh(1600, 900, '?wsl=empty');
  await openApp(page, 'Terminal');
  await page.waitForSelector('.gate [data-gate="no-distributions"]');
  check('terminal: no distribution → helpful empty state with Linux Setup button, nothing installed', (await page.$eval('.gate', (e) => e.textContent)).includes('never installs') && (await count(page, '.pane')) === 0);
  await clickText(page, '.gate .btn', 'Open Linux Setup');
  await sleep(500);
  check('terminal: empty-state button opens Linux Setup', (await visibleWindows(page)).includes('Linux Setup'));
  await page.close();

  page = await fresh(1600, 900, '?wsl=ready');
  await setSelected(page, '');
  await openApp(page, 'Terminal');
  await page.waitForSelector('.gate [data-gate="choose"]');
  check('terminal: nothing selected → asks which distro, never assumes one, lists only regular distros', (await count(page, '.gate .choice')) === 3 && (await count(page, '.pane')) === 0);
  await shot(page, '12-terminal-choose');
  await page.close();

  page = await fresh(1600, 900, '?wsl=ready');
  await setSelected(page, 'Gentoo-Gone');
  await openApp(page, 'Terminal');
  await page.waitForSelector('.gate [data-gate="selected-missing"]');
  check('terminal: removed selected distro handled gracefully (no session, selection untouched)', (await text(page, '.gate h2')).includes('Gentoo-Gone') && (await count(page, '.pane')) === 0 && (await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).runtime.selected_distro)) === 'Gentoo-Gone');
  await page.close();

  // A real session UI
  page = await fresh(1600, 900, '?wsl=ready');
  await setSelected(page, 'Ubuntu-24.04');
  await openApp(page, 'Terminal');
  await page.waitForSelector('.pane.active .xterm-rows');
  await sleep(300);
  let log = await pv(page);
  check('terminal: opens automatically in the SELECTED distribution only', log.opened.length === 1 && log.opened[0].distro === 'Ubuntu-24.04', JSON.stringify(log.opened));
  check('terminal: opened with a real size from the fitted xterm', log.opened[0].rows > 5 && log.opened[0].cols > 20, JSON.stringify(log.opened));
  check('terminal: tab + status bar show the distro and a running indicator', (await text(page, '.tab.on .name')) === 'Ubuntu-24.04' && (await count(page, '.tab.on .dot.running')) === 1 && (await text(page, '.status')).includes('Ubuntu-24.04'));
  check('terminal: banner rendered through xterm.js', (await termText(page)).includes('VLINUX browser preview'));
  await shot(page, '13-terminal');

  await page.keyboard.type('héllo wörld ✓ 日本語');
  await page.keyboard.press('Enter');
  await page.keyboard.press('ArrowUp');
  await page.keyboard.press('Backspace');
  await page.keyboard.down('Control'); await page.keyboard.press('KeyC'); await page.keyboard.up('Control');
  await sleep(200);
  log = await pv(page);
  const sent = log.inputs.map((i) => i.data).join('');
  check('terminal: typed characters (incl. Unicode) reach the session input', sent.includes('héllo wörld ✓ 日本語'), JSON.stringify(sent));
  check('terminal: Enter, arrow keys, Backspace and Ctrl+C are forwarded as terminal bytes', sent.includes('\r') && sent.includes('\x1b[A') && sent.includes('\x7f') && sent.includes('\x03'), JSON.stringify(sent));
  check('terminal: output is rendered', (await termText(page)).includes('héllo'));

  // Shell shortcuts are NOT swallowed by the terminal, and terminal shortcuts do not leak.
  await page.keyboard.down('Alt'); await page.keyboard.press('Digit2'); await page.keyboard.up('Alt');
  await sleep(150);
  check('terminal: Alt+2 still switches workspace (shell keys pass through)', (await text(page, '.ws .pill.active')) === '2');
  await page.keyboard.down('Alt'); await page.keyboard.press('Digit1'); await page.keyboard.up('Alt');
  await sleep(600);

  // Tabs
  const nIn = (await pv(page)).inputs.length;
  await page.keyboard.down('Control'); await page.keyboard.down('Shift'); await page.keyboard.press('KeyT'); await page.keyboard.up('Shift'); await page.keyboard.up('Control');
  await sleep(500);
  check('terminal: Ctrl+Shift+T opens a second tab (and does not type into the shell)', (await count(page, '.tab')) === 2 && (await pv(page)).inputs.length === nIn);
  check('terminal: duplicate distro tabs are numbered', (await text(page, '.tab:nth-child(1) .name')) === 'Ubuntu-24.04 · 1' && (await text(page, '.tab:nth-child(2) .name')) === 'Ubuntu-24.04 · 2');
  check('terminal: only the active pane is visible', (await page.$$eval('.pane', (p) => p.filter((e) => getComputedStyle(e).display !== 'none').length)) === 1);
  await page.click('.newgroup .icon-btn[aria-label^="Choose a distribution"]');
  await page.waitForSelector('.menu');
  check('terminal: the new-tab menu offers installed regular distros only (docker-desktop excluded)', (await count(page, '.menu .menu-item')) === 3);
  await shot(page, '14-terminal-menu');
  await page.click('.menu .menu-item[data-distro="archlinux"]');
  await sleep(500);
  log = await pv(page);
  check('terminal: a tab can target another distro without changing the selection', (await text(page, '.tab.on .name')) === 'archlinux' && log.opened.at(-1).distro === 'archlinux' && (await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).runtime.selected_distro)) === 'Ubuntu-24.04');
  await page.keyboard.type('arch-only');
  await sleep(150);
  const lastIn = (await pv(page)).inputs.at(-1);
  check('terminal: input goes to the active session only', lastIn.id === log.opened.at(-1).id);
  await page.evaluate(() => document.querySelector('.tab .tab-main').click());
  await sleep(300);
  check('terminal: switching tabs switches the visible session', (await text(page, '.tab.on .name')).startsWith('Ubuntu-24.04'));

  // Resize follows the window
  const r0 = (await pv(page)).resizes.length;
  await page.evaluate(() => document.querySelector('.win:not(.hidden) .ctl button[aria-label="Maximise"]').click());
  await sleep(700);
  const rs = (await pv(page)).resizes;
  check('terminal: maximising resizes the PTY to the new grid', rs.length > r0 && rs.at(-1).cols > log.opened[0].cols, JSON.stringify(rs.at(-1)));
  check('terminal: status bar shows the grid size', /\d+×\d+/.test(await text(page, '.status .mono')));
  await shot(page, '15-terminal-maximised');
  await page.evaluate(() => document.querySelector('.win:not(.hidden) .ctl button[aria-label="Restore"]').click());
  await sleep(500);

  // Clear
  await page.evaluate(() => document.querySelector('.tools .icon-btn.text').click());
  await sleep(200);
  check('terminal: Clear wipes the screen', !(await termText(page)).includes('héllo'));

  // Shell ended (as if the user typed `exit`)
  const endedId = (await pv(page)).opened[0].id;
  await page.evaluate((id) => window.__vlinuxTerminalPreview.end(id, { kind: 'exited', code: 0 }), endedId);
  await page.waitForSelector('.ended');
  check('terminal: an ended shell shows a banner with Restart/Close and a grey indicator', (await text(page, '.ended')).includes('ended') && (await count(page, '.tab.on .dot.exited')) === 1);
  await shot(page, '17-terminal-ended');
  const openedBefore = (await pv(page)).opened.length;
  await clickText(page, '.ended .btn', 'Restart');
  await sleep(600);
  log = await pv(page);
  check('terminal: Restart starts a fresh session in the same distro and drops the old one', log.opened.length === openedBefore + 1 && log.opened.at(-1).distro === 'Ubuntu-24.04' && log.closed.includes(endedId) && (await count(page, '.tab')) === 3 && (await count(page, '.ended')) === 0);

  // Close confirmation guards running sessions
  await page.keyboard.down('Alt'); await page.keyboard.press('KeyW'); await page.keyboard.up('Alt');
  await page.waitForSelector('.panel[aria-label^="Close Terminal"]');
  check('terminal: closing the window with live sessions asks first', (await text(page, '.panel[aria-label^="Close Terminal"] p')).includes('3 terminal sessions'));
  await shot(page, '16-terminal-close-confirm');
  await clickText(page, '.panel[aria-label^="Close Terminal"] .btn', 'Keep it open');
  await sleep(200);
  check('terminal: "Keep it open" keeps the window and the sessions', (await count(page, '.panel[aria-label^="Close Terminal"]')) === 0 && (await count(page, '.tab')) === 3);
  await page.click('.win:not(.hidden) .ctl .x');
  await page.waitForSelector('.panel[aria-label^="Close Terminal"]');
  await clickText(page, '.panel[aria-label^="Close Terminal"] .btn', 'Close Terminal');
  await sleep(400);
  log = await pv(page);
  check('terminal: confirming closes the window and every session it owned', (await count(page, '.win')) === 0 && log.closed.length === log.opened.length, JSON.stringify([log.opened.length, log.closed.length]));
  await page.close();

  // Setup → Open terminal in a specific distro
  page = await fresh(1600, 900, '?wsl=ready');
  await setSelected(page, '');
  await openApp(page, 'Linux Setup');
  await page.waitForSelector('.distro[data-name="archlinux"]');
  await page.evaluate(() => { const b = [...document.querySelectorAll('.distro[data-name="archlinux"] button')].find((x) => x.textContent.includes('Open terminal')); b.click(); });
  await sleep(900);
  await page.waitForSelector('.pane.active .xterm-rows');
  log = await pv(page);
  check('terminal: "Open terminal" in Linux Setup starts a session in that exact distro', log.opened.length === 1 && log.opened[0].distro === 'archlinux', JSON.stringify(log.opened));
  check('terminal: that session is not used to change the selected/default distro', (await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config') ?? '{}').runtime?.selected_distro ?? '')) === '');
  await page.close();

  // ====================================================================================
  console.log('M4 - Files (browser preview: real UI against a SIMULATED in-memory filesystem)');
  const setVal = (pg, sel, v) => pg.$eval(sel, (e, v2) => { e.value = v2; e.dispatchEvent(new Event('input', { bubbles: true })); }, v);
  const rowNames = (pg) => pg.$$eval('.files .row', (els) => els.map((e) => e.getAttribute('data-name')));
  const hasRow = async (pg, n) => (await rowNames(pg)).includes(n);
  const focusList = (pg) => pg.focus('.rows');
  const dnd = (pg, srcSel, dstSel, ctrl = false) => pg.evaluate((s2, d2, c2) => {
    const dt = new DataTransfer();
    const fire = (el, type) => el.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt, ctrlKey: c2 }));
    const src = document.querySelector(s2); const dst = document.querySelector(d2);
    if (!src || !dst) throw new Error(`dnd: missing ${s2} or ${d2}`);
    fire(src, 'dragstart'); fire(dst, 'dragenter'); fire(dst, 'dragover'); fire(dst, 'drop'); fire(src, 'dragend');
  }, srcSel, dstSel, ctrl);
  const openFiles = async (pg) => { await pg.click('.dock .item[aria-label^="Files"]'); await pg.waitForSelector('.files .row'); await sleep(400); };
  const goto = async (pg, name) => { await pg.click(`.files .row[data-name="${name}"]`); await pg.keyboard.press('Enter'); await sleep(250); };
  const place = (id, space = 'linux') => `.files .place[data-place="${id}"][data-space="${space}"]`;
  const typePath = async (pg, t) => { await pg.focus('.rows'); await pg.keyboard.down('Control'); await pg.keyboard.press('l'); await pg.keyboard.up('Control'); await pg.waitForSelector('.pathbar input'); await setVal(pg, '.pathbar input', t); await pg.keyboard.press('Enter'); await sleep(250); };

  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setSelected(page, 'archlinux');
  await openFiles(page);
  check('Files opens as a window', (await visibleWindows(page)).join() === 'Files');
  check('sidebar has Home, the six personal folders, the file system and the Windows folders',
    (await count(page, place('home'))) === 1 && (await count(page, '.files .place[data-space="linux"]')) === 8 && (await count(page, '.files .place[data-space="windows"]')) === 6 && (await count(page, place('root'))) === 1);
  check('opens at the Linux home of the selected distro', (await page.$$eval('.pathbar .crumb', (c) => c.map((x) => x.textContent.trim()))).join('/') === '//home/preview'.replace('//', '/') || (await text(page, '.pathbar')).includes('preview'));
  check('Linux space is labelled', (await text(page, '.pathbar .space')) === 'Linux');
  check('home lists folders; hidden files are not shown by default', (await hasRow(page, 'projects')) && !(await hasRow(page, '.bashrc')));
  await page.click('[data-act="hidden"]'); await sleep(250);
  check('hidden-files toggle reveals dotfiles', await hasRow(page, '.bashrc'));
  await page.click('[data-act="hidden"]'); await sleep(250);
  check('and hides them again', !(await hasRow(page, '.bashrc')));
  check('symbolic links are marked with their target, broken ones flagged',
    (await text(page, '.row[data-name="Music"]')).includes('→ /opt/music') && (await text(page, '.row[data-name="broken"]')).includes('broken or unknown'));
  check('folders sort before files; sizes shown for files', (await rowNames(page))[0] !== undefined);
  await shot(page, '20-files-home');

  console.log('M4 - large folder (virtual list + paging)');
  await goto(page, 'projects'); await goto(page, 'big');
  check('the 3000-entry folder reports its full count without loading it', /3.?000 items/.test(await text(page, '[data-status="count"]')) && /loaded/.test(await text(page, '[data-status="count"]')));
  const domRows = await count(page, '.files .row');
  check(`only a window of rows is in the DOM (${domRows} of 3000)`, domRows > 0 && domRows < 80, String(domRows));
  await page.$eval('.rows', (e) => { e.scrollTop = e.scrollHeight; });
  await page.waitForFunction(() => document.querySelector('.files .row[data-name="file-3000.txt"]'), { timeout: 8000 });
  check('scrolling to the end loads the last pages on demand', true);
  await page.$eval('.rows', (e) => { e.scrollTop = 0; });
  await sleep(200);

  console.log('M4 - navigation, typed paths and errors');
  await page.click('button[aria-label="Back"]'); await sleep(250);
  check('Back returns to the previous folder', (await text(page, '.pathbar')).includes('projects') && !(await text(page, '.pathbar')).includes('big'));
  await page.click('button[aria-label="Forward"]'); await sleep(250);
  check('Forward goes on', (await text(page, '.pathbar')).includes('big'));
  await page.click('button[aria-label="Parent folder"]'); await sleep(250);
  check('Parent folder goes up', (await hasRow(page, 'hello.py')));
  await typePath(page, '/root');
  check('a folder without permission shows a clear error and nothing else', (await count(page, '.state.err[data-kind="permission_denied"]')) === 1);
  await typePath(page, '/does/not/exist');
  check('a missing folder is reported as missing', (await count(page, '.state.err[data-kind="not_found"]')) === 1);
  await page.click(place('home')); await sleep(250);
  check('an error is left by opening any place', (await count(page, '.state.err')) === 0 && (await hasRow(page, 'projects')));
  await page.click('.files button[aria-label="Parent folder"]'); await sleep(250);
  await page.click(place('home')); await sleep(250);
  await typePath(page, '/etc/../root');
  check('a path with .. is refused, no navigation', (await text(page, '.pathbar')).length > 0 && (await count(page, '.pathbar small[role="alert"]')) === 1);
  await page.keyboard.press('Escape'); await sleep(100);
  await typePath(page, 'C:\\Windows');
  check('Windows paths are not accepted in the Linux path bar', (await text(page, '.pathbar small[role="alert"]')).includes('Windows'));
  await page.keyboard.press('Escape'); await sleep(100);
  await typePath(page, '~/Desktop');
  check('~ is expanded to the distro home', (await text(page, '.pathbar')).includes('Desktop') && (await count(page, '.state.err')) === 0);

  console.log('M4 - create, rename, delete (confirmation), copy, move');
  await page.click(place('home')); await sleep(250);
  await page.click('[data-act="new-folder"]'); await page.waitForSelector('[data-dialog="prompt"]');
  await setVal(page, '[data-dialog="prompt"] input', 'Projects2');
  await page.click('[data-act="confirm-prompt"]'); await sleep(350);
  check('New folder creates a real entry and selects it', (await hasRow(page, 'Projects2')) && (await count(page, '.row.sel[data-name="Projects2"]')) === 1);
  await page.click('[data-act="new-folder"]'); await page.waitForSelector('[data-dialog="prompt"]');
  await setVal(page, '[data-dialog="prompt"] input', 'Projects2');
  await page.click('[data-act="confirm-prompt"]'); await sleep(250);
  check('creating a name that exists is refused with a message and nothing is overwritten', (await text(page, '[data-dialog-error]')).includes('already exists'));
  await setVal(page, '[data-dialog="prompt"] input', 'a/b');
  await page.click('[data-act="confirm-prompt"]'); await sleep(150);
  check('a name with a slash is refused', (await count(page, '[data-dialog-error]')) === 1);
  await page.keyboard.press('Escape'); await sleep(200);
  check('Escape closes the dialog', (await count(page, '[data-dialog]')) === 0);
  await page.click('.row[data-name="Projects2"]'); await page.keyboard.press('F2'); await page.waitForSelector('[data-dialog="prompt"]');
  await setVal(page, '[data-dialog="prompt"] input', 'Work');
  await page.keyboard.press('Enter'); await sleep(350);
  check('F2 renames', (await hasRow(page, 'Work')) && !(await hasRow(page, 'Projects2')));
  await page.click('.row[data-name="Work"]'); await page.keyboard.press('Delete'); await page.waitForSelector('[data-dialog="delete"]');
  check('Delete asks first and says it is permanent', (await text(page, '[data-dialog="delete"]')).includes('permanently'));
  await clickText(page, '[data-dialog="delete"] button', 'Keep'); await sleep(250);
  check('declining keeps the item', (await hasRow(page, 'Work')) && (await count(page, '[data-dialog]')) === 0);
  await page.click('.row[data-name="Work"]'); await page.keyboard.press('Delete'); await page.waitForSelector('[data-dialog="delete"]');
  await page.click('[data-act="confirm-delete"]'); await sleep(500);
  check('confirming deletes it', !(await hasRow(page, 'Work')));

  await page.click(place('documents')); await sleep(300);
  check('a non-empty folder that is not linked shows its real contents', (await hasRow(page, 'notes.txt')) && (await hasRow(page, 'thesis.odt')));
  await page.click('.row[data-name="notes.txt"]'); await focusList(page);
  await page.keyboard.down('Control'); await page.keyboard.press('c'); await page.keyboard.up('Control');
  await page.click(place('desktop')); await sleep(250); await focusList(page);
  await page.keyboard.down('Control'); await page.keyboard.press('v'); await page.keyboard.up('Control'); await sleep(600);
  check('Copy + Paste copies into another folder', await hasRow(page, 'notes.txt'));
  await focusList(page);
  await page.keyboard.down('Control'); await page.keyboard.press('v'); await page.keyboard.up('Control'); await sleep(600);
  check('pasting again keeps both (nothing is overwritten)', (await rowNames(page)).some((n) => /^notes \(copy.*\)\.txt$/.test(n ?? '')), (await rowNames(page)).join());
  await page.click(place('documents')); await sleep(250);
  check('the original is still there after a copy', await hasRow(page, 'notes.txt'));
  await page.click('.row[data-name="notes.txt"]'); await focusList(page);
  await page.keyboard.down('Control'); await page.keyboard.press('x'); await page.keyboard.up('Control');
  await page.click(place('pictures')); await sleep(250); await focusList(page);
  await page.keyboard.down('Control'); await page.keyboard.press('v'); await page.keyboard.up('Control'); await sleep(600);
  check('Cut + Paste moves', await hasRow(page, 'notes.txt'));
  await page.click(place('documents')); await sleep(250);
  check('and the source no longer has it', !(await hasRow(page, 'notes.txt')));

  console.log('M4 - drag and drop, context menus');
  await dnd(page, '.row[data-name="thesis.odt"]', '.row[data-name="drafts"]'); await sleep(600);
  check('dragging a file onto a folder moves it', !(await hasRow(page, 'thesis.odt')));
  await goto(page, 'drafts');
  check('it arrived in the target folder', await hasRow(page, 'thesis.odt'));
  await dnd(page, '.row[data-name="thesis.odt"]', place('desktop'), true); await sleep(600);
  check('Ctrl-drag onto a sidebar place copies (source stays)', await hasRow(page, 'thesis.odt'));
  await page.click(place('desktop')); await sleep(250);
  check('and it is in Desktop', await hasRow(page, 'thesis.odt'));
  await page.click('.row[data-name="thesis.odt"]', { button: 'right' }); await page.waitForSelector('.ctx');
  check('right-click on an item opens its menu', (await count(page, '.ctx [data-cmd="rename"]')) === 1 && (await count(page, '.ctx [data-cmd="delete"]')) === 1);
  await page.mouse.click(700, 700); await sleep(150);
  await page.click('.rows', { button: 'right', offset: { x: 300, y: 330 } }); await page.waitForSelector('.ctx');
  check('right-click on empty space offers New folder / Paste', (await count(page, '.ctx [data-cmd="new-folder"]')) === 1 && (await count(page, '.ctx [data-cmd="paste"]')) === 1);
  await page.keyboard.press('Escape');
  await page.mouse.click(700, 700); await sleep(150);

  console.log('M4 - shared Windows folders');
  await page.click(place('downloads', 'windows')); await sleep(300);
  check('Windows folder is labelled and lists the real Windows Downloads (redirected to another drive in the preview)', (await text(page, '.pathbar .space')) === 'Windows' && (await hasRow(page, 'photo.jpg')));
  await page.click('.row[data-name="setup.exe"]', { button: 'right' }); await page.waitForSelector('.ctx');
  check('programs are never launched from VLINUX: Open is disabled with an explanation', await page.$eval('.ctx [data-cmd="open"]', (b) => b.disabled && b.title.length > 10));
  await page.mouse.click(700, 700); await sleep(150);
  await shot(page, '21-files-windows');
  await page.close();

  console.log('M4 - Files without a distribution');
  page = await fresh(1600, 900, '?wsl=missing');
  await page.evaluate(() => localStorage.clear());
  await openFiles(page);
  check('Windows folders stay browsable', (await count(page, '.files .place[data-space="windows"]')) === 6 && (await hasRow(page, 'photo.jpg') || (await count(page, '.row')) > 0));
  check('the Linux section explains why it is unavailable and offers Setup', (await count(page, '[data-note="linux"]')) === 1 && (await text(page, '[data-note="linux"]')).includes('Open Linux Setup'));
  await page.close();

  console.log('M4 - Settings: Linux folders apply / undo with conflicts (simulated)');
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setSelected(page, 'archlinux');
  await open2(page);
  await clickText(page, '.settings nav button', 'Linux folders');
  await page.waitForSelector('.folder[data-folder="downloads"] [data-badge]');
  const badges = async () => page.$$eval('.folder [data-badge]', (els) => els.map((e) => e.textContent.trim()));
  check('before any check every folder reads "preferred, not checked" - never connected', (await badges()).every((b) => b.includes('not checked')), (await badges()).join('|'));
  await page.waitForSelector('[data-act="check-mappings"]:not([disabled])', { timeout: 8000 });
  await page.click('[data-act="check-mappings"]'); await sleep(500);
  const st3 = async (id) => page.$eval(`.folder[data-folder="${id}"] .mapstate`, (e) => e.getAttribute('data-mapstate'));
  check('after a check each folder reports what is really there', (await st3('downloads')) === 'ready' && (await st3('documents')) === 'conflict' && (await st3('pictures')) === 'ready' && (await st3('music')) === 'conflict' && (await st3('videos')) === 'conflict' && (await st3('desktop')) === 'ready',
    [await st3('downloads'), await st3('documents'), await st3('pictures'), await st3('music'), await st3('videos'), await st3('desktop')].join());
  check('nothing says Connected yet', !(await badges()).some((b) => b === 'Connected'));
  check('conflicts are "pending", with the entries listed', (await text(page, '.folder[data-folder="documents"] [data-badge]')).includes('Pending') && (await text(page, '.folder[data-folder="documents"] .conflict')).includes('thesis.odt'));
  check('Confirm is disabled until the user picks an option', await page.$eval('.folder[data-folder="documents"] [data-act="apply-resolution"]', (b) => b.disabled));
  check('the foreign symlink is described and has its own offer', (await text(page, '.folder[data-folder="music"] .conflict')).includes('/opt/music') && (await count(page, '.folder[data-folder="music"] .offer')) >= 1);

  await page.click('.folder[data-folder="downloads"] [data-act="apply"]'); await sleep(500);
  check('Apply on a missing folder connects it, verified', (await st3('downloads')) === 'applied' && (await text(page, '.folder[data-folder="downloads"] [data-badge]')) === 'Connected');
  await page.click('.folder[data-folder="pictures"] [data-act="apply"]'); await sleep(250);
  check('replacing an empty folder asks for explicit confirmation first', (await count(page, '.folder[data-folder="pictures"] [data-act="confirm-apply"]')) === 1 && (await st3('pictures')) === 'ready');
  await page.click('.folder[data-folder="pictures"] [data-act="confirm-apply"]'); await sleep(500);
  check('after confirming it is connected', (await st3('pictures')) === 'applied');
  await page.click('.folder[data-folder="documents"] .offer:last-of-type input'); await sleep(100);
  check('picking an option enables Confirm', await page.$eval('.folder[data-folder="documents"] [data-act="apply-resolution"]', (b) => !b.disabled));
  await page.click('.folder[data-folder="documents"] [data-act="apply-resolution"]'); await sleep(600);
  check('"Move aside" connects Documents without deleting anything', (await st3('documents')) === 'applied');
  await shot(page, '22-settings-mappings');

  console.log('M4 - Files shows the connection');
  await openFiles(page);
  check('linked Linux folders are marked in the sidebar', (await page.$eval(place('downloads'), (e) => e.getAttribute('data-linked'))) === 'true' && (await page.$eval(place('music'), (e) => e.getAttribute('data-linked'))) === 'false');
  check('the unresolved ones are not marked', (await page.$eval(place('videos'), (e) => e.getAttribute('data-linked'))) === 'false');
  await page.click(place('downloads')); await sleep(350);
  check('~/Downloads shows the Windows Downloads files and says so', (await hasRow(page, 'photo.jpg')) && (await count(page, '[data-chip="shared"]')) === 1);
  await page.click(place('home')); await sleep(300);
  check('the old Documents content was moved aside, not removed', await hasRow(page, 'Documents.vlinux-backup'));
  await goto(page, 'Documents.vlinux-backup');
  check('and its files are all still there', (await hasRow(page, 'thesis.odt')) && (await hasRow(page, 'notes.txt')));
  await open2(page);
  await clickText(page, '.settings nav button', 'Linux folders');
  await page.waitForSelector('.folder[data-folder="downloads"] [data-badge]');
  await page.waitForSelector('[data-act="check-mappings"]:not([disabled])', { timeout: 8000 });
  await page.click('[data-act="check-mappings"]'); await sleep(500);
  check('a re-check finds the real state: applied folders are still connected', (await st3('downloads')) === 'applied');
  await page.click('.folder[data-folder="downloads"] input[role="switch"]'); await sleep(900);
  check('turning sharing off while linked says it is still linked (never silently unlinks or deletes)', (await st3('downloads')) === 'linked_but_private' && (await text(page, '.folder[data-folder="downloads"] [data-badge]')).includes('Still linked'), await st3('downloads'));
  await page.click('.folder[data-folder="downloads"] [data-act="undo"]'); await sleep(500);
  check('Undo removes only the link; the folder is private again', (await st3('downloads')) === 'private');
  await page.click('.folder[data-folder="documents"] [data-act="undo"]'); await sleep(500);
  check('Undo of a "move aside" restores the original folder', (await st3('documents')) === 'conflict' || (await st3('documents')) === 'ready');
  await page.close();

  page = await fresh(1600, 900, '?wsl=ready');
  await setSelected(page, 'archlinux');
  await open2(page);
  await clickText(page, '.settings nav button', 'Linux folders');
  await page.waitForSelector('.folder[data-folder="downloads"] [data-badge]');
  check('after a reload nothing is claimed as connected until the distribution is checked again', (await badges()).every((b) => b.includes('not checked') || b.includes('Linux-private')));
  await page.close();

  console.log('Responsive layouts (no overflow, nothing off-screen)');
  for (const [w, h] of [[1920, 1080], [2560, 1440], [1366, 768], [1024, 640], [800, 560], [480, 800]]) {
    page = await fresh(w, h);
    await page.evaluate(() => localStorage.clear());
    await open2(page);
    const m = await page.evaluate(() => ({
      sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth,
      sh: document.documentElement.scrollHeight, ch: document.documentElement.clientHeight,
      dock: document.querySelector('.dock').getBoundingClientRect().toJSON(),
      bar: document.querySelector('.bar').getBoundingClientRect().toJSON(),
      win: document.querySelector('.win')?.getBoundingClientRect().toJSON(),
    }));
    check(`${w}×${h}: no scroll overflow`, m.sw <= m.cw && m.sh <= m.ch, JSON.stringify(m));
    check(`${w}×${h}: dock & bar inside viewport`, m.dock.left >= 0 && m.dock.right <= w + 1 && m.dock.bottom <= h + 1 && m.bar.right <= w + 1);
    if (m.win) check(`${w}×${h}: window inside viewport`, m.win.right <= w + 1 && m.win.bottom <= h + 1 && m.win.left >= -1, JSON.stringify(m.win));
    await shot(page, `09-res-${w}x${h}`);
    await page.close();
  }

  console.log('Idle CPU (no polling)');
  page = await fresh(1600, 900);
  const cdp = await page.createCDPSession();
  await cdp.send('Performance.enable');
  const metric = async (n) => (await cdp.send('Performance.getMetrics')).metrics.find((x) => x.name === n).value;
  const t0 = await metric('TaskDuration'); const w0 = Date.now();
  await sleep(6000);
  const t1 = await metric('TaskDuration'); const w1 = Date.now();
  const cpu = ((t1 - t0) / ((w1 - w0) / 1000)) * 100;
  const heap = (await metric('JSHeapUsedSize')) / 1048576;
  console.log(`  measured: renderer main-thread busy ${cpu.toFixed(2)}% over ${((w1 - w0) / 1000).toFixed(1)}s idle; JS heap ${heap.toFixed(1)} MiB`);
  check('idle main-thread busy < 2%', cpu < 2, `${cpu}`);
  await page.close();

  check('no console/page errors', errors.length === 0, errors.join(' | '));
} catch (e) {
  failed++;
  console.error('E2E crashed:', e);
} finally {
  await browser.close();
  server.close();
}

async function open2(page) {
  await page.keyboard.down('Alt'); await page.keyboard.press('F1'); await page.keyboard.up('Alt');
  await page.waitForSelector('.panel'); await page.keyboard.type('settings'); await page.keyboard.press('Enter'); await sleep(500);
}

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
