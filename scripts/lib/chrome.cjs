/**
 * One browser resolver for every proof script in this app.
 *
 * WHY THIS EXISTS: /usr/bin/chromium-browser on Ubuntu 24.04 is NOT a browser — it is a
 * snap stub that prints "requires the chromium snap" and exits. Puppeteer then dies with the
 * misleading error "The browser is already running for <profile>", so any proof written as
 * `process.env.CHROME_PATH || '/usr/bin/chromium-browser'` reports the FEATURE as broken when
 * only the launcher was. Call this instead of naming a path.
 *
 * CHROME_PATH is honoured only when it points at a real browser binary.
 */
const fs = require('fs');
const { execSync } = require('child_process');

const STUB_MARKERS = ['chromium snap', 'requires the chromium snap'];

function isStub(p) {
  try {
    const size = fs.statSync(p).size;
    if (!size) return true;
    if (size > 5 * 1024 * 1024) return false;          // real binaries are big
    const head = fs.readFileSync(p, 'utf8').slice(0, 2048);
    if (/^\x7fELF/.test(head) || head.startsWith('MZ')) return false;  // real executable
    const low = head.toLowerCase();
    return STUB_MARKERS.some((m) => low.includes(m)) || size < 1024;
  } catch (e) {
    return true;
  }
}

function usable(p) {
  if (!p || !fs.existsSync(p)) return false;
  try { fs.accessSync(p, fs.constants.X_OK); } catch (e) { return false; }
  return !isStub(p);
}

function glob(pattern) {
  try {
    return execSync(`${pattern} 2>/dev/null`, { encoding: 'utf8' }).split('\n').filter(Boolean);
  } catch (e) {
    return [];
  }
}

function resolve() {
  const env = process.env.CHROME_PATH;
  if (env && usable(env)) return env;

  const candidates = [];
  // Playwright chromium first — it is the browser this box actually ships.
  candidates.push(...glob('ls -d /root/.cache/ms-playwright/chromium-*/chrome-linux64/chrome').reverse());
  candidates.push(...glob('ls -d /root/.cache/puppeteer/chrome/*/chrome-linux64/chrome').reverse());
  candidates.push(
    '/opt/google/chrome/chrome',
    '/usr/bin/google-chrome-stable',
    '/usr/bin/google-chrome',
    '/snap/bin/chromium',        // the real snap binary, if the snap is installed
    '/usr/bin/chromium',         // real package, if present
    '/usr/lib/chromium/chromium',
  );

  for (const c of candidates) {
    if (usable(c)) return c;
  }
  throw new Error('no real Chrome/Chromium binary found (the only candidate on PATH is the Ubuntu snap stub)');
}

module.exports = resolve;
module.exports.resolve = resolve;
module.exports.usable = usable;
