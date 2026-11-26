// GATT fragmentation, identical bounds to src/provisioning.rs.
export const MAX_MESSAGE_BYTES = 4096;
export const MAX_FRAGMENTS = 64;
export const HEADER_BYTES = 3;
export const MIN_ATT_PAYLOAD = 20;

export function fragment(messageId: number, message: Uint8Array, attPayload: number): Uint8Array[] {
  if (attPayload < MIN_ATT_PAYLOAD) throw new Error('payload_too_small');
  if (message.length === 0 || message.length > MAX_MESSAGE_BYTES) throw new Error('too_large');
  const chunk = attPayload - HEADER_BYTES;
  const count = Math.ceil(message.length / chunk);
  if (count > MAX_FRAGMENTS) throw new Error('too_large');
  const out: Uint8Array[] = [];
  for (let i = 0; i < count; i++) {
    const body = message.subarray(i * chunk, (i + 1) * chunk);
    const f = new Uint8Array(HEADER_BYTES + body.length);
    f.set([messageId & 0xff, i, count]);
    f.set(body, HEADER_BYTES);
    out.push(f);
  }
  return out;
}

/** Reassembles notified response fragments for one message id. */
export class Reassembler {
  private parts: (Uint8Array | undefined)[] = [];
  private count = 0;
  private bytes = 0;
  constructor(readonly messageId: number) {}

  /** Returns the complete message, null while incomplete; throws on violations. */
  push(fragment: Uint8Array): Uint8Array | null {
    if (fragment.length <= HEADER_BYTES) throw new Error('malformed');
    const [id, index, count] = fragment;
    if (id !== this.messageId) return null; // another message's fragment
    if (count === 0 || count > MAX_FRAGMENTS || index >= count) throw new Error('malformed');
    if (this.count === 0) { this.count = count; this.parts = Array.from({length: count}, () => undefined); }
    if (count !== this.count) throw new Error('inconsistent');
    if (this.parts[index]) throw new Error('duplicate');
    const body = fragment.subarray(HEADER_BYTES);
    this.bytes += body.length;
    if (this.bytes > MAX_MESSAGE_BYTES) throw new Error('too_large');
    this.parts[index] = body;
    if (this.parts.some(p => !p)) return null;
    const out = new Uint8Array(this.bytes);
    let o = 0;
    for (const p of this.parts as Uint8Array[]) { out.set(p, o); o += p.length; }
    return out;
  }
}
