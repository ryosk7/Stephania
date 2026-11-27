// Enforces the code architecture of DESIGN.md §7:
//   app → features → shared (one direction), no feature imports another,
//   src/app holds routes only, features are entered through their index.
import {readFileSync, readdirSync, statSync} from 'node:fs';
import {dirname, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {describe, expect, it} from 'vitest';

const SRC = dirname(fileURLToPath(import.meta.url));

function files(dir: string): string[] {
  return readdirSync(dir).flatMap(n => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? files(p) : /\.(ts|tsx)$/.test(n) ? [p] : [];
  });
}

/** Import specifiers resolved to a path under src/ ('' for packages and native modules). */
function imports(file: string): {spec: string; target: string}[] {
  const text = readFileSync(file, 'utf8');
  return [...text.matchAll(/(?:import|export)\s[^'"]*?from\s+['"]([^'"]+)['"]/g)].map(m => {
    const spec = m[1]!;
    const target = spec.startsWith('@/') ? join(SRC, spec.slice(2)) : spec.startsWith('.') ? resolve(dirname(file), spec) : '';
    return {spec, target: target && relative(SRC, target)};
  });
}

const layer = (p: string) => p.split('/')[0];
const feature = (p: string) => (p.startsWith('features/') ? p.split('/')[1] : undefined);
const all = files(SRC).filter(f => !f.endsWith('architecture.test.ts') && !f.endsWith('.d.ts')).map(f => ({file: relative(SRC, f), imports: imports(f)}));

describe('architecture (DESIGN.md §7)', () => {
  it('has the three layers only', () => {
    expect([...new Set(all.map(f => layer(f.file)))].sort()).toEqual(['app', 'features', 'shared']);
  });

  it('shared never imports features or app', () => {
    const bad = all.filter(f => layer(f.file) === 'shared').flatMap(f => f.imports.filter(i => ['features', 'app'].includes(layer(i.target)!)).map(i => `${f.file} → ${i.spec}`));
    expect(bad).toEqual([]);
  });

  it('features never import app or another feature', () => {
    const bad = all.filter(f => layer(f.file) === 'features').flatMap(f => f.imports
      .filter(i => layer(i.target) === 'app' || (feature(i.target) && feature(i.target) !== feature(f.file)))
      .map(i => `${f.file} → ${i.spec}`));
    expect(bad).toEqual([]);
  });

  it('routes use features only through their public index, and src/app has routes only', () => {
    const routes = all.filter(f => layer(f.file) === 'app');
    const deep = routes.flatMap(f => f.imports.filter(i => layer(i.target) === 'features' && i.target.split('/').length > 2).map(i => `${f.file} → ${i.spec}`));
    expect(deep).toEqual([]);
    // Every file in src/app is a route: a default export, no tests or helpers.
    const notRoutes = routes.filter(f => !/export (default\b|\{\s*default\b)/.test(readFileSync(join(SRC, f.file), 'utf8')));
    expect(notRoutes.map(f => f.file)).toEqual([]);
  });

  it('stories are colocated with the component they show', () => {
    const orphans = all.filter(f => /\.stories\.tsx$/.test(f.file))
      .filter(f => !all.some(o => o.file === f.file.replace(/\.stories\.tsx$/, '.tsx')));
    expect(orphans.map(f => f.file)).toEqual([]);
  });

  it('tests are colocated with the file they test', () => {
    const orphans = all.filter(f => /\.test\.ts$/.test(f.file))
      .filter(f => !all.some(o => o.file === f.file.replace(/\.test\.ts$/, '.ts') || o.file === f.file.replace(/\.test\.ts$/, '.tsx')));
    expect(orphans.map(f => f.file)).toEqual([]);
  });
});
