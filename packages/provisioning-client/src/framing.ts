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
