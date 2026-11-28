import {resolve, sep} from 'node:path';
import {describe, expect, it} from 'vitest';
import {staticPath} from './static-path.mjs';

const root = resolve('/srv/storybook-static');

describe('staticPath', () => {
  it('serves files inside the root', () => {
    expect(staticPath(root, '/iframe.html?id=x')).toBe(`${root}${sep}iframe.html`);
    expect(staticPath(root, '/assets/a%20b.js')).toBe(`${root}${sep}assets${sep}a b.js`);
  });

  it('refuses traversal, including encoded separators', () => {
    // Plain dot segments are normalised by URL parsing and stay inside.
    expect(staticPath(root, '/../etc/passwd')).toBe(`${root}${sep}etc${sep}passwd`);
    expect(staticPath(root, '/%2e%2e/etc/passwd')).toBe(`${root}${sep}etc${sep}passwd`);
    for (const url of ['/..%2f..%2fetc%2fpasswd', '/..%2Fetc%2Fpasswd', '/a/..%2f..%2f..%2fetc%2fpasswd',
      '/..%5c..%5cetc', '/%00', '/%E0%A4%A', '/']) {
      expect(staticPath(root, url), url).toBeNull();
    }
  });
});
