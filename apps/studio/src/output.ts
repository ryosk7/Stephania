// Stream destination (PUT/GET/DELETE /api/v1/output). The control plane
// validates again; these checks only give an immediate, specific message.
import type {OutputView} from './api/client';

export const YOUTUBE_URL = 'rtmps://a.rtmps.youtube.com/live2';
const URL_RE = /^rtmps:\/\/([A-Za-z0-9.-]{1,253})(?::([0-9]{1,5}))?\/[A-Za-z0-9._~-]{1,128}$/;
const LABEL_RE = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/;
const KEY_RE = /^[A-Za-z0-9._~-]{8,256}$/;

/** A problem with the entered destination, or null when it looks valid. Never quotes the key. */
export function outputProblem(url: string, key: string): string | null {
  const m = URL_RE.exec(url.trim());
  const host = m?.[1] ?? '';
  const port = m?.[2] === undefined ? 443 : Number(m[2]);
  if (!m || !host.includes('.') || /^[0-9.]+$/.test(host) || !host.split('.').every(l => LABEL_RE.test(l)) || port < 1 || port > 65535) {
    return `Server URL must look like ${YOUTUBE_URL} (RTMPS, without the stream key)`;
  }
  if (!KEY_RE.test(key.trim())) return 'Stream key must be 8 to 256 letters, digits, "-", "_", "." or "~"';
  return null;
}

/** One line for the current state; admins also see the server and the key's last 4 characters. */
export function outputSummary(s: OutputView | undefined): string {
  if (!s) return 'Loading…';
  if (!s.configured) return 'Not configured: streaming cannot start';
  if ('key_last4' in s && s.key_last4) return `Configured: ${s.url} · key ending ${s.key_last4}`;
  return 'Configured';
}
