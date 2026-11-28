// Maps a request URL to a file under `root`, or null. Used by the
// Storybook smoke server: the path is decoded once and must stay inside
// root after resolution, so `..`, `%2e%2e`, an encoded `/` (`%2f`) or `\`
// cannot escape it; malformed escapes and NUL bytes are refused.
import {resolve, sep} from 'node:path';

export function staticPath(root, url) {
  let path;
  try {
    path = decodeURIComponent(new URL(url, 'http://x').pathname);
  } catch {
    return null;
  }
  if (path.includes('\0') || path.includes('\\')) return null;
  const base = resolve(root);
  const full = resolve(base, `.${path}`);
  return full.startsWith(base + sep) ? full : null;
}
