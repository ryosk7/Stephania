// Builds dist/index.html: one self-contained file, no external resources.
// The inline script is pinned by a CSP sha256 hash, and SHA256SUMS lets users
// verify the page they open.
import {createHash} from 'node:crypto';
import {mkdirSync, readFileSync, writeFileSync} from 'node:fs';

const read = p => readFileSync(new URL(p, import.meta.url), 'utf8');
const lib = read('./node_modules/qrcode-generator/qrcode.js');
const licence = '/* qrcode-generator 1.4.4 — Copyright (c) 2009 Kazuhiko Arase — MIT License */\n';
export const script = [licence + lib, read('./src/logic.js'), read('./src/ui.js')].join('\n;\n');
if (script.includes('</script')) throw new Error('script must not contain </script');
const hash = 'sha256-' + createHash('sha256').update(script).digest('base64');
const html = read('./src/template.html').replace('__SCRIPT_HASH__', hash).replace('__SCRIPT__', () => script);
mkdirSync(new URL('./dist/', import.meta.url), {recursive: true});
writeFileSync(new URL('./dist/index.html', import.meta.url), html);
writeFileSync(new URL('./dist/script.js', import.meta.url), script);
const sum = createHash('sha256').update(html).digest('hex');
writeFileSync(new URL('./dist/SHA256SUMS', import.meta.url), `${sum}  index.html\n`);
console.log(`dist/index.html ${html.length} bytes, sha256 ${sum}`);
