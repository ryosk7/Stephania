import {describe, expect, it} from 'vitest';
import {ApiError, createClient} from './client';

const respond = (status: number, body?: unknown) => async () =>
  new Response(body === undefined ? null : JSON.stringify(body), {status, headers: {'Content-Type': 'application/json'}});

describe('api client', () => {
  it('sends bearer token and JSON body', async () => {
    let seen: {url: string; init?: RequestInit} | undefined;
    const c = createClient(() => 'tok', async (url, init) => { seen = {url, init}; return respond(200, {id: 'lt', revision: 2, document: {}})(); });
    await c.saveScene('lt', 1, {version: 1, name: 'LT', layers: []});
    expect(seen?.url).toBe('/api/v1/scenes/lt');
    expect(seen?.init?.method).toBe('PUT');
    expect((seen?.init?.headers as Record<string, string>).Authorization).toBe('Bearer tok');
    expect(JSON.parse(String(seen?.init?.body))).toEqual({revision: 1, document: {version: 1, name: 'LT', layers: []}});
  });
  it('raises the error envelope', async () => {
    const c = createClient(() => 't', respond(501, {error: {code: 'not_implemented', message: 'Program unchanged', request_id: 'abc'}}));
    const e = await c.transition('lt', 1).catch(x => x);
    expect(e).toBeInstanceOf(ApiError);
    expect([e.status, e.code, e.requestId]).toEqual([501, 'not_implemented', 'abc']);
  });
  it('handles non-JSON errors and 204', async () => {
    const e = await createClient(() => 't', async () => new Response('<h1>x</h1>', {status: 502})).status().catch(x => x);
    expect(e.code).toBe('http_502');
    await expect(createClient(() => 't', respond(204)).deleteScene('lt', 1)).resolves.toBeUndefined();
  });
  it('escapes ids', async () => {
    let url = '';
    await createClient(() => 't', async u => { url = u; return respond(200, [])(); }).revisions('a/b');
    expect(url).toBe('/api/v1/scenes/a%2Fb/revisions');
  });
});
