// Run an `async page => {...}` function file against the installed desktop WebView over CDP.
// The desktop must have been started with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>.
// Usage: node tools/cdp_run.mjs [function.js] [--url-prefix http://tauri.localhost] [--port 14222]
// Without a function file it prints the open pages, which doubles as a connectivity check.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { URL } from 'node:url';

const require = createRequire(new URL('../apps/web/package.json', import.meta.url));
const { chromium } = require('@playwright/test');
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
};
const file = args.find(
  (value, index) => !value.startsWith('--') && !args[index - 1]?.startsWith('--'),
);
const port = option('--port', '14222');
const prefix = option('--url-prefix', 'http://tauri.localhost');
const print = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 15000 });
try {
  const pages = browser.contexts().flatMap((context) => context.pages());
  const page = pages.find((candidate) => candidate.url().startsWith(prefix)) ?? pages[0];
  if (!page) throw new Error('no page found over CDP');
  if (file === undefined) {
    print({ url: page.url(), title: await page.title(), pages: pages.map((p) => p.url()) });
  } else {
    const source = readFileSync(file, 'utf8');
    const run = (0, eval)(`(${source.trim().replace(/;\s*$/u, '')})`);
    print(await run(page));
  }
} finally {
  await browser.close().catch(() => {});
}
