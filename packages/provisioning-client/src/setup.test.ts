import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {describe, expect, it} from 'vitest';
import {parseSetupUri, toHex} from './index';

const f: {valid: {uri: string; secret: string}[]; invalid: string[]} =
  JSON.parse(readFileSync(join(__dirname, '../../../contracts/fixtures/ble/setup-uris.json'), 'utf8'));

describe('contracts/fixtures/ble/setup-uris.json', () => {
  it('accepts valid URIs', () => {
    for (const v of f.valid) expect(toHex(parseSetupUri(v.uri)!), v.uri).toBe(v.secret);
  });
  it('rejects invalid URIs', () => {
    for (const u of f.invalid) expect(parseSetupUri(u), u).toBeNull();
  });
});
