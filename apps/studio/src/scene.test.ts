import {readdirSync, readFileSync} from 'node:fs';
import {join} from 'node:path';
import {describe, expect, it} from 'vitest';
import {validateSceneText} from './scene';

const dir = join(__dirname, '../../../contracts/fixtures/scenes');
const fixtures = (kind: string) => readdirSync(join(dir, kind)).filter(f => f.endsWith('.json')).sort()
  .map(f => [f, readFileSync(join(dir, kind, f), 'utf8')] as const);

describe('shared scene fixtures (same suite as Rust and Ruby)', () => {
  it('has fixtures', () => {
    expect(fixtures('valid').length).toBeGreaterThan(0);
    expect(fixtures('invalid').length).toBeGreaterThan(0);
  });
  it.each(fixtures('valid'))('accepts %s', (_, text) => expect(validateSceneText(text).violation).toBeNull());
  it.each(fixtures('invalid'))('rejects %s', (_, text) => expect(validateSceneText(text).violation).not.toBeNull());
});
