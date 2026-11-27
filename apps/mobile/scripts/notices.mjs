// Third-party notices for T3Box Connect: every package that ships in the
// app (production dependencies, transitively) with its licence and the
// licence text found in the package, deduplicated. Shown on the Licences
// screen; the bundled fonts' OFL-1.1 text is among them (docs/LICENSING.md).
//   node scripts/notices.mjs          write src/features/settings/licences.json
//   node scripts/notices.mjs --check  fail if that file is out of date
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync, readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {join} from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const out = join(root, 'src/features/settings/licences.json');
const LICENCE_FILE = /^(licen[cs]e|copying|notice|ofl)([-._].*)?$/i;

const tree = JSON.parse(execFileSync('npm', ['ls', '--omit=dev', '--all', '--long', '--json'], {cwd: root, maxBuffer: 1 << 28, encoding: 'utf8'}));
const seen = new Map();
(function walk(deps) {
  for (const [name, d] of Object.entries(deps ?? {})) {
    if (!d.path || d.extraneous) continue;
    const key = `${name}@${d.version}`;
    if (!seen.has(key)) seen.set(key, {name, version: d.version, path: d.path});
    walk(d.dependencies);
  }
})(tree.dependencies);

const texts = {};
const packages = [];
for (const p of [...seen.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))) {
  if (p.name.startsWith('@t3box/')) continue; // our own code (MIT, LICENSE)
  const pkg = JSON.parse(readFileSync(join(p.path, 'package.json'), 'utf8'));
  const license = typeof pkg.license === 'string' ? pkg.license : pkg.license?.type ?? (pkg.licenses ?? []).map(l => l.type ?? l).join(' OR ') ?? '';
  const files = existsSync(p.path) ? readdirSync(p.path).filter(f => LICENCE_FILE.test(f)).sort() : [];
  const text = files.map(f => readFileSync(join(p.path, f), 'utf8').trim()).join('\n\n').replace(/\r\n/g, '\n');
  let ref = null;
  if (text) {
    ref = createHash('sha256').update(text).digest('hex').slice(0, 16);
    texts[ref] = text;
  }
  packages.push({name: p.name, version: p.version, license: license || 'UNKNOWN', text: ref});
}

const json = JSON.stringify({packages, texts}, null, 1) + '\n';
if (process.argv.includes('--check')) {
  const current = existsSync(out) ? readFileSync(out, 'utf8') : '';
  if (current !== json) {
    console.error('licences.json is out of date: run `npm run notices` in apps/mobile');
    process.exit(1);
  }
  const missing = packages.filter(p => !p.text).map(p => `${p.name}@${p.version} (${p.license})`);
  console.log(`notices: ${packages.length} packages, ${Object.keys(texts).length} licence texts; without a licence file: ${missing.length}`);
} else {
  writeFileSync(out, json);
  console.log(`wrote ${out}: ${packages.length} packages, ${(json.length / 1024).toFixed(0)} KiB`);
}
