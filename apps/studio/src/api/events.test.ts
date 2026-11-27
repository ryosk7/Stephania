import {describe, expect, it} from 'vitest';
import {SseParser, subscribeEvents, type ServerEvent} from './events';

describe('SseParser', () => {
  it('parses events split across arbitrary chunk boundaries', () => {
    const stream = 'retry: 2000\n\n: keepalive\n\nevent: snapshot\ndata: {"engine":null,"scenes":[]}\n\nevent: scene.updated\r\ndata: {"id":"lt","revision":2}\r\n\r\n';
    for (let size = 1; size <= stream.length; size += 1) {
      const p = new SseParser();
      const out: ServerEvent[] = [];
      for (let i = 0; i < stream.length; i += size) out.push(...p.push(stream.slice(i, i + size)));
      expect(out).toEqual([
        {event: 'snapshot', data: {engine: null, scenes: []}},
        {event: 'scene.updated', data: {id: 'lt', revision: 2}},
      ]);
      expect(p.retryMs).toBe(2000);
    }
  });

  it('joins multi-line data and defaults the event name', () => {
    expect(new SseParser().push('data: a\ndata: b\n\n')).toEqual([{event: 'message', data: 'a\nb'}]);
  });
});

function streamResponse(chunks: string[], status = 200): Response {
  const body = new ReadableStream<Uint8Array>({
    start(c) {
      const enc = new TextEncoder();
      for (const chunk of chunks) c.enqueue(enc.encode(chunk));
      c.close();
    },
  });
  return new Response(body, {status, headers: {'Content-Type': 'text/event-stream'}});
}

describe('subscribeEvents', () => {
  it('sends the bearer token, delivers events and reconnects after EOF', async () => {
    const calls: RequestInit[] = [];
    const events: ServerEvent[] = [];
    let resolveSecond!: () => void;
    const second = new Promise<void>(r => (resolveSecond = r));
    const stop = subscribeEvents({
      token: () => 'tok',
      maxDelayMs: 1,
      onEvent: e => events.push(e),
      fetcher: async (_i, init) => {
        calls.push(init!);
        if (calls.length === 2) resolveSecond();
        return streamResponse(['event: snapshot\ndata: {"n":', `${calls.length}}\n\n`]);
      },
    });
    await second;
    await new Promise(r => setTimeout(r, 5));
    stop();
    expect((calls[0].headers as Record<string, string>).Authorization).toBe('Bearer tok');
    expect(events.slice(0, 2)).toEqual([{event: 'snapshot', data: {n: 1}}, {event: 'snapshot', data: {n: 2}}]);
  });

  it('stops retrying on 401', async () => {
    let count = 0;
    const states: string[] = [];
    subscribeEvents({
      token: () => 'bad',
      maxDelayMs: 1,
      onEvent: () => {},
      onState: (s, d) => states.push(`${s}:${d ?? ''}`),
      fetcher: async () => { count += 1; return new Response('{}', {status: 401}); },
    });
    await new Promise(r => setTimeout(r, 30));
    expect(count).toBe(1);
    expect(states).toContain('closed:http_401');
  });
});
