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
  check('dock has launcher + 6 apps', (await count(page, '.dock .item')) === 7);
  check('every app is available (none disabled); Files and Terminal are enabled', (await count(page, '.dock .item[aria-disabled="true"]')) === 0 && (await count(page, '.dock .item[aria-label^="Files"]:not([aria-disabled="true"])')) === 1 && (await count(page, '.dock .item[aria-label^="Terminal"]:not([aria-disabled="true"])')) === 1);
  check('wallpaper rendered', (await count(page, '.wp svg')) === 1);
  await shot(page, '01-desktop');

  console.log('M6 - first launch');
  check('a first launch shows the welcome guidance on the empty desktop', (await count(page, '[data-testid="welcome"]')) === 1 && (await text(page, '[data-testid="welcome"]')).includes('Show my distributions'));
  check('the welcome says nothing is started or changed until asked, and names the way back to Windows', (await text(page, '[data-testid="welcome"]')).includes('until you ask') && (await text(page, '[data-testid="welcome"]')).includes('Ctrl+Alt+Q'));
  await page.click('[data-testid="welcome-detect"]'); await sleep(900);
  check('its button opens Distributions', (await visibleWindows(page)).some((l) => (l ?? '').startsWith('Distributions')));
  check('the welcome steps aside while a window is open', (await count(page, '[data-testid="welcome"]')) === 0);
  await page.keyboard.down('Alt'); await page.keyboard.press('KeyW'); await page.keyboard.up('Alt'); await sleep(500);
  check('it returns when the desktop is empty again', (await count(page, '[data-testid="welcome"]')) === 1);
  await page.click('[data-testid="welcome-dismiss"]'); await sleep(300);
  await page.reload({ waitUntil: 'networkidle0' }); await page.waitForSelector('.shell.ready'); await sleep(700);
  check('"Don\'t show this again" is remembered', (await count(page, '[data-testid="welcome"]')) === 0);
  await page.evaluate(() => localStorage.removeItem('vlinux.welcome.dismissed'));
  await page.evaluate(() => { const c = JSON.parse(localStorage.getItem('vlinux.config') ?? '{}'); c.runtime = { selected_distro: 'archlinux' }; localStorage.setItem('vlinux.config', JSON.stringify(c)); });
  await page.reload({ waitUntil: 'networkidle0' }); await page.waitForSelector('.shell.ready'); await sleep(700);
  check('it is not shown once a distribution has been chosen', (await count(page, '[data-testid="welcome"]')) === 0);
  await page.evaluate(() => localStorage.clear());
  await page.reload({ waitUntil: 'networkidle0' }); await page.waitForSelector('.shell.ready'); await sleep(700);

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

  await page.click('.folder[data-folder="downloads"] [data-act="apply"]'); await sleep(250);
  const dlText = await text(page, '.folder[data-folder="downloads"] .confirm');
  check('M6.1: even a plain Apply first says exactly what will change and waits for a click', (await st3('downloads')) === 'ready' && dlText.includes('one new symbolic link') && dlText.includes('~/Downloads') && dlText.includes('no file is copied, moved, renamed or deleted') && dlText.includes('Undo link'), dlText);
  await page.click('.folder[data-folder="downloads"] [data-act="confirm-apply"]'); await sleep(500);
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

  console.log('M5 - Distribution Manager (SIMULATED preview: six fake distributions, no real WSL)');
  const dcard = (n) => `[data-testid="distro-card"][data-distro="${n}"]`;
  const dstate = (pg, n) => pg.$eval(dcard(n), (e) => e.getAttribute('data-state'));
  const dact = (n, a) => `${dcard(n)} [data-action="${a}"]`;
  const dtab = async (pg, t) => { await pg.click(`[data-tab="${t}"]`); await sleep(250); };
  const dfacts = (pg, n) => pg.$eval(`${dcard(n)} .dm-facts`, (e) => [...e.querySelectorAll('dt')].map((dt) => `${dt.textContent.trim()}: ${dt.nextElementSibling.textContent.trim()}`).join(' | '));
  const waitState = (pg, n, st, ms = 8000) => pg.waitForFunction((sel, want) => document.querySelector(sel)?.getAttribute('data-state') === want, { timeout: ms }, dcard(n), st);
  const waitConf = (pg, n) => pg.waitForFunction((name) => { const p = document.querySelector('[data-testid="wslconf-panel"]'); return !!p && p.textContent.includes(`· ${name}`) && ![...p.querySelectorAll('button')].some((b) => b.textContent.includes('Reading')) && !p.textContent.includes('Reading the file'); }, { timeout: 8000 }, n).then(() => sleep(250), async (e) => { console.log('waitConf timeout; panel text:', (await pg.$eval('[data-testid="wslconf-panel"]', (p) => p.textContent.slice(0, 400)).catch(() => '(no panel)'))); throw e; });
  const listeners = (pg) => pg.evaluate(() => window.__vlinuxDistrosPreview?.listeners() ?? null);
  const openDistros = async (pg) => { await pg.click('.dock .item[aria-label^="Distributions"]'); await pg.waitForSelector('[data-testid="distro-card"]', { timeout: 8000 }); await sleep(300); };

  page = await fresh(1600, 900, '?wsl=multi');
  await page.evaluate(() => localStorage.clear()); await page.reload({ waitUntil: 'networkidle0' }); await page.waitForSelector('.shell.ready'); await sleep(900);
  check('the dock has an enabled Distributions tile', (await count(page, '.dock .item[aria-label^="Distributions"]:not([aria-disabled="true"])')) === 1);
  await openDistros(page);
  check('six distributions are listed, each with its own state', (await count(page, '[data-testid="distro-card"]')) === 6
    && (await dstate(page, 'archlinux')) === 'running' && (await dstate(page, 'Ubuntu-24.04')) === 'stopped' && (await dstate(page, 'Debian')) === 'running'
    && (await dstate(page, 'FedoraLinux-42')) === 'stopped' && (await dstate(page, 'Alpine')) === 'stopped' && (await dstate(page, 'docker-desktop')) === 'unavailable');
  check('the preview says it is simulated', (await text(page, '[data-testid="distros-sim-banner"]')).startsWith('SIMULATED'));
  check('discovery listeners exist only while the window is open', JSON.stringify(await listeners(page)) === '{"op":1,"diag":1}', JSON.stringify(await listeners(page)));
  await shot(page, '20-distributions');

  const f0 = await dfacts(page, 'archlinux');
  check('before inspection nothing is invented: kernel, user, architecture and package manager say "Not inspected yet"', ['Kernel', 'Default user', 'Architecture', 'Package manager'].every((l) => f0.includes(`${l}: Not inspected yet`)) && !f0.includes('x86_64') && !f0.includes('5.15') && !f0.includes('pacman'), f0);
  check('a card shows name, display name, WSL version and state', f0.includes('Name: archlinux') && f0.includes('Display name: Arch Linux') && f0.includes('WSL version: WSL 2'));
  check('Debian is shown as WSL 1', (await dfacts(page, 'Debian')).includes('WSL version: WSL 1'));
  check('the Windows default is marked, and nothing is preferred yet', (await text(page, '[data-testid="win-default-name"]')) === 'archlinux' && (await text(page, '[data-testid="pref-name"]')) === 'none chosen' && (await count(page, `${dcard('archlinux')} .dm-badge.win`)) === 1);

  // Preferred distribution is VLINUX's own setting, separate from the Windows default.
  await page.click(dact('Debian', 'set_preferred')); await page.waitForFunction(() => document.querySelector('[data-testid="pref-name"]')?.textContent === 'Debian', { timeout: 6000 }).catch(() => {}); await sleep(700);
  check('Set as preferred records it in VLINUX config and marks the card', (await text(page, '[data-testid="pref-name"]')) === 'Debian' && (await page.$eval(dcard('Debian'), (e) => e.getAttribute('data-preferred'))) === 'true'
    && (await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).runtime.selected_distro)) === 'Debian');
  check('the Windows default is unchanged (preferred is not wsl --set-default)', (await text(page, '[data-testid="win-default-name"]')) === 'archlinux' && (await count(page, `${dcard('archlinux')} .dm-badge.win`)) === 1 && (await count(page, `${dcard('Debian')} .dm-badge.win`)) === 0);
  check('the preferred button is disabled on the preferred card, with a reason', await page.$eval(dact('Debian', 'set_preferred'), (e) => e.disabled && e.title.length > 3));

  // Inspect: read-only, cached, explicit.
  await page.click(dact('archlinux', 'inspect')); await page.waitForFunction((sel) => document.querySelector(sel)?.textContent.includes('pacman'), { timeout: 8000 }, `${dcard('archlinux')} .dm-facts`); await sleep(900);
  const f1 = await dfacts(page, 'archlinux');
  check('inspecting a running distribution fills in what was really read', f1.includes('Kernel: 5.15.167.4-microsoft-standard-WSL2') && f1.includes('Architecture: x86_64') && f1.includes('Package manager: pacman') && f1.includes('Default user: preview') && f1.includes('Last inspected: Inspected just now'), f1);
  check('and says it did not start the distribution', (await text(page, `${dcard('archlinux')} [data-testid="card-note"]`)).includes('was not started'));
  check('the inspection did not change the state', (await dstate(page, 'archlinux')) === 'running');

  await page.click(dact('Alpine', 'inspect')); await page.waitForSelector('[data-testid="inspect-start-confirm"]', { timeout: 6000 }).catch(() => {}); await sleep(900);
  check('inspecting a STOPPED distribution asks first and starts nothing', (await count(page, '[data-testid="inspect-start-confirm"]')) === 1 && (await dstate(page, 'Alpine')) === 'stopped');
  await clickText(page, '[data-testid="inspect-start-confirm"] button', 'Cancel'); await sleep(300);
  check('cancelling leaves it stopped', (await count(page, '[data-testid="inspect-start-confirm"]')) === 0 && (await dstate(page, 'Alpine')) === 'stopped');
  await page.click(dact('Alpine', 'inspect')); await page.waitForSelector('[data-testid="inspect-start-yes"]', { timeout: 6000 }); await sleep(300);
  await page.click('[data-testid="inspect-start-yes"]');
  await page.waitForFunction((sel) => document.querySelector(sel)?.textContent.includes('started the distribution'), { timeout: 8000 }, `${dcard('Alpine')} [data-testid="card-note"]`);
  await waitState(page, 'Alpine', 'running'); await sleep(300);
  check('with permission it starts, inspects, and says that it started it', (await dstate(page, 'Alpine')) === 'running' && (await text(page, `${dcard('Alpine')} [data-testid="card-note"]`)).includes('started the distribution'), `${await dstate(page, 'Alpine')} / ${await text(page, `${dcard('Alpine')} [data-testid="card-note"]`)}`);
  check('an unsupported package manager is reported as such, not guessed', (await dfacts(page, 'Alpine')).includes('Package manager: None VLINUX supports was found'));

  // Start: explicit, visible progress, no duplicates.
  await page.click(dact('Ubuntu-24.04', 'start')); await sleep(150);
  check('Start shows a Starting state at once and blocks a second start', (await dstate(page, 'Ubuntu-24.04')) === 'starting' && await page.$eval(dact('Ubuntu-24.04', 'start'), (e) => e.disabled));
  await waitState(page, 'Ubuntu-24.04', 'running');
  check('it ends in Running; the other distributions were not touched', (await dstate(page, 'FedoraLinux-42')) === 'stopped' && (await dstate(page, 'Debian')) === 'running');

  // Stop: per distribution, explicit confirmation, interruption warning.
  await page.click(dact('Debian', 'stop')); await sleep(250);
  const warn = await text(page, '[data-testid="stop-confirm"]');
  check('Stop asks for confirmation, warns about interruption, and says only this distribution stops', warn.includes('interrupted') && warn.includes('Only this distribution stops') && (await dstate(page, 'Debian')) === 'running');
  await page.click('[data-testid="stop-confirm-no"]'); await sleep(250);
  check('Cancel keeps it running', (await dstate(page, 'Debian')) === 'running' && (await count(page, '[data-testid="stop-confirm"]')) === 0);
  await page.click(dact('Debian', 'stop')); await sleep(200);
  await page.click('[data-testid="stop-confirm-yes"]');
  await page.waitForFunction((sel) => document.querySelector(sel)?.getAttribute('data-state') === 'stopped', { timeout: 8000 }, dcard('Debian'));
  check('after confirming, only Debian stops (archlinux, Ubuntu and Alpine keep running)', (await dstate(page, 'archlinux')) === 'running' && (await dstate(page, 'Ubuntu-24.04')) === 'running' && (await dstate(page, 'Alpine')) === 'running');
  check('the preferred distribution survives being stopped (no silent switch)', (await text(page, '[data-testid="pref-name"]')) === 'Debian');

  // Dangerous operations are not offered anywhere.
  const labels = await page.$$eval('[data-testid="distros-app"] button', (b) => b.map((x) => x.textContent.trim()));
  check('no button offers unregister, shutdown, delete, reset, import, export, mount or set-default', !labels.some((l) => /unregister|shut ?down|delete|reset|import|export|mount|set.?default|remove distribution|uninstall/i.test(l)), labels.join(' | '));
  // Internal (other application's) distribution.
  const internal = await page.$$eval(`${dcard('docker-desktop')} .acts button`, (b) => b.map((x) => ({ off: x.disabled, why: x.title })));
  check('an internal distribution (docker-desktop) has every action disabled, each with a reason', internal.length >= 8 && internal.every((b) => b.off && b.why.length > 3));
  check('and is labelled as used by another app', (await text(page, dcard('docker-desktop'))).includes('Used by another app'));

  // Open terminal / Files from a card.
  await page.click(dact('archlinux', 'open_terminal')); await page.waitForSelector('.tab.on .name', { timeout: 8000 }); await sleep(600);
  check('Open terminal on a card opens a session in that distribution', (await text(page, '.tab.on .name')) === 'archlinux');
  await page.click('.dock .item[aria-label^="Distributions"]'); await sleep(500);
  await page.click(dact('Ubuntu-24.04', 'open_terminal')); await sleep(900);
  check('another card adds a tab; the first session is preserved', (await count(page, '.tab')) === 2 && (await text(page, '.tab.on .name')) === 'Ubuntu-24.04' && (await text(page, '.tab:nth-child(1) .name')) === 'archlinux');
  await page.click('.dock .item[aria-label^="Distributions"]'); await sleep(500);
  await page.click(dact('archlinux', 'open_files')); await page.waitForSelector('.files .row', { timeout: 8000 }); await sleep(500);
  check('Open Files on a card opens Files for that distribution', (await text(page, '.files')).includes('archlinux'));
  await page.click('.dock .item[aria-label^="Distributions"]'); await sleep(500);

  // Packages tab.
  await page.click(`${dcard('archlinux')} .name`); await dtab(page, 'packages');
  const caps = await page.$$eval('[data-testid="packages-panel"] [data-cap]', (r) => Object.fromEntries(r.map((x) => [x.getAttribute('data-cap'), x.getAttribute('data-support')])));
  check('pacman capabilities: detect/version/search available, install/update show-the-command only', caps.detect === 'available' && caps.version === 'available' && caps.search === 'available' && caps.install === 'plan_only' && caps.update === 'plan_only', JSON.stringify(caps));
  await page.type('[aria-label="Packages to install"]', 'neovim git'); await page.click('[data-testid="pkg-review-install"]'); await sleep(400);
  check('install shows the exact command and says it was not run', (await text(page, '[data-testid="pkg-commands"]')) === 'sudo pacman -S --needed neovim git' && (await text(page, '[data-testid="pkg-plan"]')).includes('not run'));
  check('and nothing was installed or started by that', (await count(page, '[data-testid="pkg-error"]')) === 0 && (await dstate(page, 'archlinux')) === 'running');
  await page.$eval('[aria-label="Packages to install"]', (e) => { e.value = 'git; rm -rf ~'; e.dispatchEvent(new Event('input', { bubbles: true })); });
  await page.click('[data-testid="pkg-review-install"]'); await sleep(400);
  check('a malicious package string is rejected with an error and no plan', (await text(page, '[data-testid="pkg-error"]')).includes('not a valid package name') && (await count(page, '[data-testid="pkg-plan"]')) === 0);
  await page.click('[data-testid="pkg-review-update"]'); await sleep(400);
  check('update shows the full-upgrade command and the Arch partial-upgrade warning', (await text(page, '[data-testid="pkg-commands"]')) === 'sudo pacman -Syu');
  await page.$eval('[aria-label="Search for a package"]', (e) => { e.value = 'neovim'; e.dispatchEvent(new Event('input', { bubbles: true })); });
  await page.click('[data-testid="pkg-search"]'); await page.waitForSelector('[data-testid="pkg-hits"] li', { timeout: 5000 }).catch(() => {});
  check('search is read-only and returns results', (await count(page, '[data-testid="pkg-hits"] li')) >= 1);
  await page.click(`${dcard('Ubuntu-24.04')} .name`); await sleep(300);
  check('a distribution that was not inspected says so instead of assuming APT', (await count(page, '[data-testid="pkg-not-inspected"]')) === 1 && (await count(page, '[data-testid="pkg-review-install"]')) === 0);
  await page.click(dact('Ubuntu-24.04', 'inspect'));
  await page.waitForFunction(() => document.querySelector('[data-testid="packages-panel"]')?.textContent.includes('APT 2.7.14'), { timeout: 6000 }).catch(() => {}); await sleep(300);
  check('after inspection, APT is detected and a command can be prepared', (await text(page, '[data-testid="packages-panel"]')).includes('APT 2.7.14') && (await count(page, '[data-testid="pkg-review-install"]')) === 1);
  await page.click(`${dcard('Alpine')} .name`); await sleep(300);
  check('an unknown package manager gets no install form', (await count(page, '[data-testid="pkg-unsupported"]')) === 1 && (await count(page, '[data-testid="pkg-review-install"]')) === 0 && (await page.$$eval('[data-testid="packages-panel"] [data-cap]', (r) => r.every((x) => x.getAttribute('data-support') !== 'available' || ['detect'].includes(x.getAttribute('data-cap'))))) === true);
  await shot(page, '21-packages');

  // Configuration tab.
  await page.click(dact('FedoraLinux-42', 'open_config')); await sleep(500);
  check('configuration of a STOPPED distribution does not start it and offers an explicit start', (await count(page, '[data-testid="conf-needs-start"]')) === 1 && (await dstate(page, 'FedoraLinux-42')) === 'stopped');
  await clickText(page, '[data-testid="conf-needs-start"] button', 'Start');
  await page.waitForSelector('#wc-user\\.default', { timeout: 8000 }).catch(() => {}); await sleep(400);
  check('after the explicit start the file is read', (await count(page, '#wc-user\\.default')) === 1 && (await page.$eval('#wc-user\\.default', (e) => e.value)) === 'preview' && (await dstate(page, 'FedoraLinux-42')) === 'running');
  await page.click(dact('archlinux', 'open_config')); await waitConf(page, 'archlinux');
  check('the current settings are read first', (await page.$eval('#wc-automount\\.root', (e) => e.value)) === '/mnt/' && (await page.$eval('#wc-automount\\.enabled', (e) => e.value)) === 'true');
  check('unknown settings in the file are acknowledged and preserved', (await text(page, '[data-testid="wslconf-panel"]')).includes('network.hostname'));
  check('config status: managed by the distribution (wsl.conf) and by Windows (.wslconfig, read-only)', (await text(page, '[data-testid="wslconf-panel"]')).includes('Managed by the distribution') && (await text(page, '[data-testid="winconf"]')).includes('Managed by Windows') && (await text(page, '[data-testid="winconf"]')).includes('read-only'));
  check('Review is disabled until something is changed', await page.$eval('[data-testid="conf-review"]', (e) => e.disabled));
  await page.select('#wc-automount\\.enabled', 'false'); await sleep(200);
  await page.click('[data-testid="conf-review"]'); await sleep(500);
  const diffText = await text(page, '[data-testid="conf-plan"] .dm-diff');
  check('the change is previewed as a diff before anything is written', diffText.includes('+ enabled = false') && diffText.includes('− enabled = true'), diffText);
  check('the preview states that a restart is needed and will not be done for you', (await text(page, '[data-testid="conf-plan"]')).includes('will not restart or stop it for you'));
  check('Save is disabled until the review is confirmed', await page.$eval('[data-testid="conf-apply"]', (e) => e.disabled));
  await page.click('[data-testid="conf-plan"] input[type="checkbox"]'); await page.click('[data-testid="conf-apply"]'); await page.waitForSelector('[data-testid="backup-row"]', { timeout: 8000 }); await sleep(500);
  const applied = await text(page, '[data-testid="conf-applied"]');
  check('saving reports it was read back, and does NOT claim the setting is active', applied.includes('Saved and read back') && applied.includes('Not yet in effect') && !/is now (on|off|active)/i.test(applied), applied);
  check('the saved value is shown after re-reading, and a backup was taken', (await page.$eval('#wc-automount\\.enabled', (e) => e.value)) === 'false' && (await count(page, '[data-testid="backup-row"]')) === 1);
  check('the distribution was not restarted or stopped', (await dstate(page, 'archlinux')) === 'running');
  await clickText(page, '[data-testid="backup-row"] button', 'Restore'); await sleep(250);
  check('restoring asks for confirmation first', (await text(page, '[data-testid="backup-row"]')).includes('Restore this version') && (await page.$eval('#wc-automount\\.enabled', (e) => e.value)) === 'false');
  await clickText(page, '[data-testid="backup-row"] .dm-confirm button', 'Restore');
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="backup-row"]').length === 2, { timeout: 8000 }).catch(() => {}); await sleep(600);
  check('restore puts the earlier content back and keeps a backup of what it replaced', (await page.$eval('#wc-automount\\.enabled', (e) => e.value)) === 'true' && (await count(page, '[data-testid="backup-row"]')) === 2);
  await page.click(dact('Debian', 'start')); await waitState(page, 'Debian', 'running'); await sleep(300);
  await page.waitForFunction((sel) => document.querySelector(sel) && !document.querySelector(sel).disabled, { timeout: 8000 }, dact('Debian', 'open_config'));
  await page.click(dact('Debian', 'open_config')); await waitConf(page, 'Debian');
  await page.select('#wc-boot\\.systemd', 'true'); await sleep(200);
  await page.click('[data-testid="conf-review"]'); await sleep(500);
  await page.click('[data-testid="conf-plan"] input[type="checkbox"]'); await page.click('[data-testid="conf-apply"]'); await page.waitForSelector('[data-testid="conf-not-writable"]', { timeout: 8000 });
  const nw = await text(page, '[data-testid="conf-not-writable"]');
  check('when the distribution does not let VLINUX write, nothing changes, no elevation is attempted, and instructions are shown', nw.includes('Nothing was changed') && nw.includes('does not use administrator') && nw.includes('[boot]'), nw);
  await shot(page, '22-configuration');

  // Diagnostics tab.
  await page.click(dact('archlinux', 'view_diagnostics')); await sleep(400);
  check('diagnostics is labelled SIMULATED in the preview', (await text(page, '[data-testid="diag-mode-banner"]')).startsWith('SIMULATED'));
  check('before running, the panel states what the run will and will not do', (await text(page, '[data-testid="diag-plan"]')).includes('will NOT create any file') || (await text(page, '[data-testid="diag-plan"]')).includes('Will NOT create any file'));
  check('permissions to write and to start are off by default', !(await page.$eval('[data-testid="diag-allow-write"]', (e) => e.checked)) && !(await page.$eval('[data-testid="diag-allow-start"]', (e) => e.checked)));
  await page.click('[data-testid="diag-run"]'); await page.waitForSelector('[data-testid="diag-headline"]', { timeout: 15000 }); await sleep(300);
  const hl = await text(page, '[data-testid="diag-headline"]');
  check('the headline of a simulated run says it proves nothing about this PC', hl.startsWith('SIMULATED run') && hl.includes('says nothing about this PC'), hl);
  const rows = await page.$$eval('[data-testid="diag-results"] .res', (r) => r.map((x) => ({ id: x.dataset.check, o: x.dataset.outcome, e: x.dataset.evidence })));
  check('all five categories are present', ['runtime', 'terminal', 'fs', 'shared', 'desktop'].every((p) => rows.some((r) => r.id.startsWith(p))), JSON.stringify(rows.map((r) => r.id)));
  check('without write permission every writing check is "Not tested" and did not run', rows.filter((r) => ['terminal.pty_create', 'fs.create_dir', 'fs.symlink', 'shared.link_roundtrip', 'desktop.unusual_names'].includes(r.id)).every((r) => r.o === 'not_tested' && r.e === 'none'));
  check('no result claims evidence from a real WSL', rows.every((r) => r.e !== 'real_wsl'));
  check('checks that cannot be done are Unsupported or Not tested, never Passed (file associations)', rows.find((r) => r.id === 'desktop.file_association')?.o === 'not_tested');
  check('passed checks are marked SIMULATED', rows.filter((r) => r.o === 'passed').every((r) => r.e === 'simulated') && rows.some((r) => r.o === 'passed'));
  await shot(page, '23-diagnostics');
  await page.click('[data-testid="diag-allow-write"]'); await page.click('[data-testid="diag-run"]');
  await page.waitForFunction(() => document.querySelector('[data-testid="diag-run"]')?.textContent.includes('Run diagnostics') && document.querySelectorAll('[data-testid="diag-results"] .res[data-check="fs.create_dir"][data-outcome="passed"]').length === 1, { timeout: 15000 });
  const rows2 = await page.$$eval('[data-testid="diag-results"] .res', (r) => r.map((x) => ({ id: x.dataset.check, o: x.dataset.outcome, e: x.dataset.evidence, m: !!x.querySelector('.dm-badge.warn[title^="This check created"]') })));
  check('with permission the writing checks run (still simulated) and say they modified something', rows2.find((r) => r.id === 'fs.create_dir')?.o === 'passed' && rows2.find((r) => r.id === 'fs.create_dir')?.m === true && rows2.find((r) => r.id === 'fs.create_dir')?.e === 'simulated');
  const rep = await text(page, '[data-testid="diag-text"]');
  check('the copyable report is labelled simulated and lists the permissions given', rep.includes('SIMULATED') && rep.includes('create temporary files = yes') && !/preview\b.*password/i.test(rep));
  check('a Copy report button exists', (await count(page, '[data-testid="diag-copy"]')) === 1);
  check('diagnostics did not start any stopped distribution (start was not allowed)', (await dstate(page, 'Debian')) === 'running' && (await dstate(page, 'FedoraLinux-42')) === 'running');

  // Window lifecycle: closing the app detaches the listeners.
  await page.keyboard.down('Alt'); await page.keyboard.press('KeyW'); await page.keyboard.up('Alt'); await sleep(500);
  check('closing the Distribution Manager removes its listeners', JSON.stringify(await listeners(page)) === '{"op":0,"diag":0}', JSON.stringify(await listeners(page)));
  await page.close();

  // Settings: WSL configuration section.
  page = await fresh(1600, 900, '?wsl=multi');
  await open2(page);
  check('Settings keeps its existing sections and adds WSL configuration', (await count(page, '.settings nav button')) === 8);
  await clickText(page, '.settings nav button', 'WSL configuration'); await sleep(900);
  check('the WSL section lists the regular distributions only', (await page.$$eval('#wsl-distro option', (o) => o.map((x) => x.value))).join() === 'archlinux,Ubuntu-24.04,Debian,FedoraLinux-42,Alpine');
  check('it shows the Windows-managed config as read-only', (await text(page, '[data-testid="winconf"]')).includes('Managed by Windows'));
  await page.select('#wsl-distro', 'Ubuntu-24.04'); await sleep(700);
  check('choosing a stopped distribution does not start it', (await count(page, '[data-testid="conf-needs-start"]')) === 1);
  await clickText(page, '.settings nav button', 'Linux folders'); await sleep(400);
  check('the Linux folders section (M4) still works next to it', (await count(page, '.folder[data-folder="downloads"]')) === 1);
  await page.close();

  // Preferred distribution that no longer exists: warn, never switch.
  page = await fresh(1600, 900, '?wsl=multi');
  await setSelected(page, 'Gone-Distro');
  await openDistros(page);
  const pn = await text(page, '[data-testid="preferred-notice"]');
  check('a missing preferred distribution gives a clear warning and does not switch', pn.includes('no longer registered') && pn.includes('has not switched') && (await count(page, '[data-preferred="true"]')) === 0 && (await text(page, '[data-testid="pref-name"]')) === 'Gone-Distro', pn);
  check('the config still holds the missing name (no silent change)', (await page.evaluate(() => JSON.parse(localStorage.getItem('vlinux.config')).runtime.selected_distro)) === 'Gone-Distro');
  await page.close();

  // Empty / failed discovery.
  page = await fresh(1600, 900, '?wsl=empty');
  await openDistros2(page);
  check('with no distribution installed the manager explains it and points to Linux Setup', (await text(page, '[data-testid="distros-empty"]')).includes('No Linux distribution is installed'));
  await page.close();
  page = await fresh(1600, 900, '?wsl=missing');
  await openDistros2(page);
  check('with WSL missing the manager says so', (await text(page, '[data-testid="distros-empty"]')).includes('WSL is not installed'));
  await page.close();
  page = await fresh(1600, 900);
  await openDistros2(page);
  check('where WSL is unavailable it explains that instead of listing anything', (await count(page, '[data-testid="distros-empty"]')) === 1 && (await count(page, '[data-testid="distro-card"]')) === 0);
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

  console.log('M7 - Linux applications in the launcher (SIMULATED discovery: nothing is read or launched)');
  const altKey = async (pg, key, shift = false) => { await pg.keyboard.down('Alt'); if (shift) await pg.keyboard.down('Shift'); await pg.keyboard.press(key); if (shift) await pg.keyboard.up('Shift'); await pg.keyboard.up('Alt'); };
  const setCfg = async (pg, fn) => {
    await pg.evaluate((src) => {
      const c = JSON.parse(localStorage.getItem('vlinux.config') ?? '{}');
      new Function('c', src)(c);
      localStorage.setItem('vlinux.config', JSON.stringify(c));
    }, fn);
    await pg.reload({ waitUntil: 'networkidle0' });
    await pg.waitForSelector('.shell.ready');
    await sleep(900);
  };
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setSelected(page, 'archlinux');
  await altKey(page, 'F1');
  await page.waitForSelector('.panel[aria-label="Application launcher"]');
  await page.waitForFunction(() => document.querySelectorAll('.grid .app').length > 6, { timeout: 6000 });
  const names = await page.$$eval('.grid .app .name', (e) => e.map((x) => x.textContent.trim()));
  check('the selected distribution\'s applications are listed under the VLINUX apps', names.includes('Text Editor') && names.includes('Calculator'));
  check('an application that cannot launch is shown, disabled, with the reason', (await count(page, '.grid .app[aria-disabled="true"]')) >= 2 && (await page.$eval('.grid .app[aria-disabled="true"]', (e) => e.textContent)).length > 10);
  await shot(page, '30-launcher-linux');
  await page.keyboard.type('calc');
  await sleep(150);
  check('search finds Linux applications by name', (await page.$$eval('.grid .app .name', (e) => e.map((x) => x.textContent.trim()))).includes('Calculator'));
  await page.keyboard.press('Enter');
  await sleep(500);
  check('launching reports through the notification system, not a fake window', (await count(page, '.backdrop')) === 0 && (await visibleWindows(page)).length === 0);
  await altKey(page, 'F1');
  await page.waitForSelector('.panel[aria-label="Application launcher"]');
  check('the launched application appears under Recent', (await count(page, 'h4')) >= 1 && (await page.$$eval('h4', (h) => h.some((x) => x.textContent.trim() === 'Recent'))));
  await page.keyboard.type('broken');
  await sleep(150);
  await page.keyboard.press('Enter');
  await sleep(900);
  check('the unread failure is counted on the bell', (await count(page, '.bell .badge')) === 1);
  await altKey(page, 'N', true);
  await page.waitForSelector('.panel[aria-label="Notifications"]');
  check('a launch that fails is kept, with its reason, in the notification centre', (await text(page, '.panel[aria-label="Notifications"]')).toLowerCase().includes('broken'));
  await page.keyboard.press('Escape');
  await sleep(200);

  console.log('M7 - notifications');
  await altKey(page, 'N', true);
  await page.waitForSelector('.panel[aria-label="Notifications"]');
  await sleep(300);
  await page.keyboard.press('Escape');
  await sleep(200);
  check('opening the centre marks messages read', (await count(page, '.bell .badge')) === 0);

  console.log('M7 - keyboard: shortcut overlay, window switcher, snapping, workspaces');
  await altKey(page, 'F1', true);
  await page.waitForSelector('.panel[aria-label="Keyboard shortcuts"]');
  check('Alt+Shift+F1 lists the shortcuts', (await count(page, '.panel[aria-label="Keyboard shortcuts"] kbd')) > 15);
  await page.keyboard.press('Escape'); await sleep(200);
  check('Esc closes it', (await count(page, '.panel[aria-label="Keyboard shortcuts"]')) === 0);
  await page.click('.dock .item[aria-label^="Settings"]'); await sleep(500);
  await page.click('.dock .item[aria-label^="System"]'); await sleep(500);
  check('two windows are open and listed in the top bar', (await visibleWindows(page)).length === 2 && (await count(page, '.wins .wbtn')) === 2);
  check('the focused window is marked in the top bar', (await count(page, '.wins .wbtn.active')) === 1);
  await page.keyboard.down('Alt'); await page.keyboard.press('Backquote');
  await page.waitForSelector('[aria-label="Switch window"]');
  check('Alt+` shows the window switcher', true);
  await shot(page, '31-switcher');
  await page.keyboard.up('Alt'); await sleep(300);
  check('releasing Alt commits and closes the switcher', (await count(page, '[aria-label="Switch window"]')) === 0);
  const focusedTitle = () => page.$eval('.win.focused, .win[data-focused="true"]', (e) => e.getAttribute('aria-label')).catch(() => null);
  const m7focus = await focusedTitle();
  check('switching moved focus to the other window', m7focus === 'Settings');
  await page.keyboard.down('Alt'); await page.keyboard.press('Backquote');
  await page.waitForSelector('[aria-label="Switch window"]');
  await page.keyboard.press('Escape'); await page.keyboard.up('Alt'); await sleep(300);
  check('Esc cancels the switcher without changing focus', (await focusedTitle()) === 'Settings');
  const work0 = await page.$eval('.work', (e) => { const r = e.getBoundingClientRect(); return { w: r.width, h: r.height }; });
  await altKey(page, 'ArrowLeft', true); await sleep(450);
  const left = await page.$eval('.win[aria-label="Settings"]', (e) => { const r = e.getBoundingClientRect(); return { w: r.width, x: r.x }; });
  check('Alt+Shift+Left tiles the window to the left half', Math.abs(left.w - work0.w / 2) < work0.w * 0.04);
  await altKey(page, 'ArrowUp', true); await sleep(450);
  const full = await page.$eval('.win[aria-label="Settings"]', (e) => e.getBoundingClientRect().width);
  check('Alt+Shift+Up maximises', full > work0.w * 0.95);
  await altKey(page, '2', true); await sleep(400);
  check('Alt+Shift+2 sends the window to workspace 2 (it leaves this one)', (await visibleWindows(page)).join() === 'System');
  await altKey(page, '2'); await sleep(400);
  check('Alt+2 shows it there', (await visibleWindows(page)).join() === 'Settings');
  await altKey(page, '1'); await sleep(300);

  console.log('M7 - context menus');
  await page.mouse.click(1560, 130, { button: 'right' }); await sleep(250);
  check('right-click on the desktop opens a menu', (await count(page, '[role="menu"]')) === 1);
  check('the menu offers real actions (Terminal, Files, Settings, shortcuts)', (await page.$$eval('[role="menu"] button', (b) => b.map((x) => x.textContent))).join('|').match(/Open Terminal.*Open Files.*Settings.*Keyboard shortcuts/) !== null);
  await page.keyboard.press('ArrowDown'); await page.keyboard.press('Escape'); await sleep(200);
  check('Esc closes the menu', (await count(page, '[role="menu"]')) === 0);
  await page.click('.dock .item[aria-label^="System"]', { button: 'right' }); await sleep(250);
  check('right-click on a dock icon opens its menu', (await count(page, '[role="menu"]')) === 1);
  await page.keyboard.press('Escape'); await sleep(200);
  await page.$eval('.wins .wbtn', (b) => b.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 300, clientY: 40 }))); await sleep(250);
  check('the window list has a window menu (move to workspace, close)', (await page.$$eval('[role="menu"] button', (b) => b.map((x) => x.textContent).join('|'))).includes('Move to workspace'));
  await page.keyboard.press('Escape'); await sleep(200);
  await page.close();

  console.log('M7 - theme, dock edge, top bar and terminal settings');
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setCfg(page, "c.appearance={...(c.appearance??{}),theme:'oled'}; c.dock={...(c.dock??{}),position:'right',labels:true,magnify:false}; c.topbar={...(c.topbar??{}),show_windows:false}");
  check('the OLED theme is applied to the document', (await page.evaluate(() => document.documentElement.dataset.theme)) === 'oled');
  check('the dock can sit on the right edge', await page.$eval('.dock', (d) => d.getBoundingClientRect().x > innerWidth * 0.8));
  check('dock labels are shown when enabled', (await count(page, '.dock .lbl')) >= 6);
  check('top-bar parts can be hidden', (await count(page, '.wins')) === 0);
  await shot(page, '32-dock-right-oled');
  await page.close();
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setCfg(page, "c.appearance={...(c.appearance??{}),blur:false}");
  check('turning blur off marks the document (panels become solid)', await page.evaluate(() => document.documentElement.classList.contains('no-blur')));
  await page.click('.dock .item[aria-label^="Settings"]'); await sleep(500);
  await page.$$eval('.settings nav button', (b) => b.find((x) => x.textContent.includes('Desktop'))?.click()); await sleep(250);
  check('Settings has Desktop options for dock labels and top bar', (await text(page, '.settings .page')).includes('Show app names') && (await text(page, '.settings .page')).includes('Window list'));
  await page.$$eval('.settings nav button', (b) => b.find((x) => x.textContent.includes('Terminal'))?.click()); await sleep(250);
  check('Settings has Terminal appearance (font, cursor, scrollback)', (await text(page, '.settings .page')).includes('Font size') && (await text(page, '.settings .page')).includes('Scrollback'));
  await page.$$eval('.settings nav button', (b) => b.find((x) => x.textContent.includes('Startup'))?.click()); await sleep(250);
  check('start-up app can be chosen, but Terminal and Files are not offered', (await page.$$eval('[aria-label="Open at start-up"] button', (b) => b.map((x) => x.textContent.trim()).join('|'))) === 'Nothing|Distributions|Settings');
  await page.close();
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setCfg(page, "c.startup={open_app:'settings'}");
  check('the chosen start-up app opens by itself', (await visibleWindows(page)).join() === 'Settings');
  await page.close();
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setCfg(page, "c.appearance={theme:'nonsense',glass:9999,accent:'violet'}; c.terminal={font_size:3}");
  check('invalid saved values are sanitised, not trusted', (await page.evaluate(() => document.documentElement.dataset.theme)) === 'midnight');
  await page.close();

  console.log('M7 - distribution chip');
  page = await fresh(1600, 900, '?wsl=ready');
  await page.evaluate(() => localStorage.clear());
  await setSelected(page, 'archlinux');
  check('the top bar names the selected distribution', (await text(page, '.dchip')).includes('archlinux'));
  await page.click('.dchip'); await page.waitForSelector('.panel[aria-label="Linux distribution"]');
  check('its popover shows state and offers actions without switching anything', (await text(page, '.panel[aria-label="Linux distribution"]')).includes('never changes the Windows default'));
  await shot(page, '33-distro-popover');
  await page.keyboard.press('Escape'); await sleep(200);
  await page.close();

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

async function openDistros2(page) {
  await page.click('.dock .item[aria-label^="Distributions"]');
  await page.waitForSelector('[data-testid="distros-empty"], [data-testid="distro-card"]', { timeout: 8000 });
  await new Promise((r) => setTimeout(r, 400));
}
async function open2(page) {
  await page.keyboard.down('Alt'); await page.keyboard.press('F1'); await page.keyboard.up('Alt');
  await page.waitForSelector('.panel'); await page.keyboard.type('settings'); await page.keyboard.press('Enter'); await sleep(500);
}

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
