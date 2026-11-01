// Launches a headless Chromium for UI tests/screenshots.
// Uses CHROME_PATH if set; otherwise the Chromium bundled in @sparticuz/chromium (an
// npm devDependency). That build needs its bundled shared libraries on LD_LIBRARY_PATH
// when the host lacks NSS; we unpack them to the OS temp dir on first use.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import puppeteer from 'puppeteer-core';

const require = createRequire(import.meta.url);

export async function launch() {
  let exe = process.env.CHROME_PATH;
  let args = ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'];
  if (!exe) {
    const { default: chromium } = await import('@sparticuz/chromium');
    exe = await chromium.executablePath();
    args = chromium.args;
    const bin = path.join(path.dirname(require.resolve('@sparticuz/chromium')), '..', 'bin');
    const libs = path.join(os.tmpdir(), 'vlinux-al2023');
    if (!fs.existsSync(path.join(libs, 'lib', 'libnss3.so'))) {
      fs.mkdirSync(libs, { recursive: true });
      const tar = path.join(os.tmpdir(), 'vlinux-al2023.tar');
      fs.writeFileSync(tar, zlib.brotliDecompressSync(fs.readFileSync(path.join(bin, 'al2023.tar.br'))));
      execFileSync('tar', ['-xf', tar, '-C', libs]);
    }
    process.env.LD_LIBRARY_PATH = `${path.join(libs, 'lib')}:${process.env.LD_LIBRARY_PATH ?? ''}`;
  }
  return puppeteer.launch({ executablePath: exe, args, headless: 'shell', defaultViewport: null });
}
