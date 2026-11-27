// Server-Sent Events from GET /api/v1/events (contracts/openapi.yaml).
// EventSource cannot send the bearer token and tokens never go into URLs, so
// the stream is read with fetch() and parsed here.

export type ServerEvent = {event: string; data: unknown};

/** Incremental text/event-stream parser (WHATWG subset: event, data, retry). */
export class SseParser {
  private buffer = '';
  private event = '';
  private data: string[] = [];
  retryMs: number | undefined;

  /** Feeds decoded text; returns the events completed by it. */
  push(text: string): ServerEvent[] {
    this.buffer += text;
    const out: ServerEvent[] = [];
    let nl: number;
    while ((nl = this.buffer.search(/\r\n|\r|\n/)) >= 0) {
      const line = this.buffer.slice(0, nl);
      const width = this.buffer.startsWith('\r\n', nl) ? 2 : 1;
      // A lone trailing CR may be the first half of CRLF: wait for more.
      if (width === 1 && this.buffer[nl] === '\r' && nl === this.buffer.length - 1) break;
      this.buffer = this.buffer.slice(nl + width);
      if (line === '') {
        if (this.data.length > 0) {
          const raw = this.data.join('\n');
          let data: unknown = raw;
          try { data = JSON.parse(raw); } catch { /* non-JSON data stays a string */ }
          out.push({event: this.event || 'message', data});
        }
        this.event = '';
        this.data = [];
        continue;
      }
      if (line.startsWith(':')) continue;
      const colon = line.indexOf(':');
      const field = colon < 0 ? line : line.slice(0, colon);
      let value = colon < 0 ? '' : line.slice(colon + 1);
      if (value.startsWith(' ')) value = value.slice(1);
      if (field === 'event') this.event = value;
      else if (field === 'data') this.data.push(value);
      else if (field === 'retry' && /^\d+$/.test(value)) this.retryMs = Number(value);
    }
    return out;
  }
}

export type ConnectionState = 'connecting' | 'open' | 'closed';

export type EventsOptions = {
  /** Bearer token; empty means the session cookie (sent by the browser). */
  token: () => string;
  onEvent: (e: ServerEvent) => void;
  onState?: (s: ConnectionState, detail?: string) => void;
  fetcher?: (input: string, init?: RequestInit) => Promise<Response>;
  /** Upper bound for reconnect backoff. */
  maxDelayMs?: number;
};

/**
 * Keeps one event stream open, reconnecting with capped exponential
 * backoff. Every (re)connect starts with a `snapshot` event, so callers
 * reconcile from it instead of assuming nothing was missed. Returns stop().
 */
export function subscribeEvents(opts: EventsOptions): () => void {
  const fetcher = opts.fetcher ?? ((i, n) => fetch(i, n));
  const maxDelay = opts.maxDelayMs ?? 15000;
  const controller = new AbortController();
  let stopped = false;
  let failures = 0;

  // Resolves to the server's retry hint, or 'stop' when retrying cannot
  // help (bad token): the caller resubscribes when the token changes.
  async function once(): Promise<number | undefined | 'stop'> {
    opts.onState?.('connecting');
    const res = await fetcher('/api/v1/events', {
      headers: {...(opts.token() ? {Authorization: 'Bearer ' + opts.token()} : {}), Accept: 'text/event-stream'},
      credentials: 'same-origin',
      signal: controller.signal,
      cache: 'no-store',
    });
    if (!res.ok || !res.body) {
      opts.onState?.('closed', 'http_' + res.status);
      return res.status === 401 || res.status === 403 ? 'stop' : undefined;
    }
    opts.onState?.('open');
    failures = 0;
    const parser = new SseParser();
    const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
    for (;;) {
      const {value, done} = await reader.read();
      if (done) break;
      for (const e of parser.push(value)) opts.onEvent(e);
    }
    opts.onState?.('closed', 'eof');
    return parser.retryMs;
  }

  (async () => {
    while (!stopped) {
      let retry: number | undefined;
      try {
        const r = await once();
        if (r === 'stop') break;
        retry = r;
      } catch (e) {
        if (stopped) break;
        opts.onState?.('closed', String(e));
      }
      failures += 1;
      const delay = Math.min(maxDelay, Math.max(retry ?? 0, 500 * 2 ** Math.min(failures - 1, 5)));
      await new Promise(r => setTimeout(r, delay));
    }
  })();

  return () => {
    stopped = true;
    controller.abort();
  };
}
