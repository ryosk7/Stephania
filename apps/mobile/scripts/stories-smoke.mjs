// Renders every story of the built web Storybook in headless Chrome, once
// per theme, and fails on any page error or empty render. Screenshots go to
// <out> for review (uploaded by CI).
//   node scripts/stories-smoke.mjs storybook-static <out-dir>
import {createServer} from 'node:http';
import {mkdirSync, readFileSync, statSync} from 'node:fs';
import {extname, join} from 'node:path';
import {chromium} from 'playwright-core';
import {staticPath} from './static-path.mjs';

const [dir = 'storybook-static', out = 'storybook-shots'] = process.argv.slice(2);
mkdirSync(out, {recursive: true});
const types = {'.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.ttf': 'font/ttf', '.woff2': 'font/woff2', '.png': 'image/png'};
// Loopback only, and only files under <dir>: the server is up for the
// whole run and must not expose the developer's machine to the LAN.
const server = createServer((req, res) => {
  const path = staticPath(dir, req.url);
  if (!path || !statSync(path, {throwIfNoEntry: false})?.isFile()) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, {'content-type': types[extname(path)] ?? 'application/octet-stream'});
  res.end(readFileSync(path));
});
await new Promise((ok, fail) => server.once('error', fail).listen(0, '127.0.0.1', ok));
const base = `http://127.0.0.1:${server.address().port}`;

const stories = Object.values(JSON.parse(readFileSync(join(dir, 'index.json'), 'utf8')).entries).filter(e => e.type === 'story');
const browser = await chromium.launch({executablePath: process.env.CHROME_PATH ?? '/usr/bin/google-chrome', args: ['--no-sandbox']});
const page = await browser.newPage({viewport: {width: 420, height: 800}});
const failures = [];
let errors = [];
page.on('pageerror', e => errors.push(String(e)));
for (const s of stories) {
  for (const theme of ['Light', 'Dark']) {
    errors = [];
    await page.goto(`${base}/iframe.html?id=${s.id}&viewMode=story`);
    await page.getByRole('radio', {name: theme}).first().click({timeout: 15000}).catch(e => errors.push(`no theme switch: ${e.message}`));
    await page.waitForTimeout(400);
    const box = await page.getByTestId('story').boundingBox().catch(() => null);
    if (!box || box.height < 8) errors.push('rendered nothing');
    await page.screenshot({path: join(out, `${s.id}--${theme.toLowerCase()}.png`)});
    if (errors.length) failures.push(`${s.id} (${theme}): ${errors.join('; ')}`);
  }
}
await browser.close();
server.close();
console.log(`${stories.length} stories × 2 themes rendered`);
if (failures.length) { console.error(failures.join('\n')); process.exit(1); }
