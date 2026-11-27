// Management API client for an enrolled Stephania (docs/AUTH.md, contracts/openapi.yaml).
// Every request goes through the native pinned client: TLS must chain to the
// device CA whose SHA-256 the device delivered sealed over BLE. There is no
// fallback to the system trust store or to plain HTTP.
import type {ManagementEnrollment} from '@stephania/provisioning-client';

export type NativePinned = {
  request(url: string, method: string, headersJson: string, body: string | null, caSha256: string, timeoutMs: number):
    Promise<{status: number; headers: string; body: string}>;
};

export class DeviceError extends Error {
  constructor(readonly code: string, message: string, readonly status?: number) {
    super(message);
  }
}

export type Status = {state: string; streaming_requested: boolean; last_error: string; program: {scene_id: string; revision: number} | null; engine_restarted?: boolean; /** Unix ms the running native worker started (engine.status), null while none runs. */ native_worker_started_unix_ms?: number | null};
/** Layer of a stored scene (docs/SCENE.md); v1 layers have no `type` (video). */
export type SceneLayer = {
  id: string; type?: 'video' | 'image' | 'color' | 'text' | 'template'; source?: string;
  x: number; y: number; width: number; height: number; visible: boolean; color?: string;
};
export type StoredScene = {id: string; revision: number; document: {name: string; version: number; layers?: SceneLayer[]; canvas?: {background?: string}}};
export type AudioInput = {id: 'hdmi' | 'mic'; enabled: boolean; device: string; channels: number; gain_db: number; muted: boolean};

export const TIMEOUT_MS = 8000;

export class DeviceClient {
  private preferred = 0;

  constructor(private readonly enrollment: ManagementEnrollment, private readonly native: NativePinned) {
    if (!/^[0-9a-f]{64}$/.test(enrollment.caSha256)) throw new DeviceError('bad_enrollment', 'invalid CA fingerprint');
    if (!enrollment.urls.length || enrollment.urls.some(u => !u.startsWith('https://'))) throw new DeviceError('bad_enrollment', 'enrollment URLs must be https');
  }

  /** URL that answered last (shown to the user, e.g. for "open on PC"). */
  get baseUrl(): string {
    return this.enrollment.urls[this.preferred]!;
  }

  async call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
    const urls = this.enrollment.urls;
    let lastError: unknown;
    // Try the URL that worked last first, then the others (name, then addresses).
    const order = [this.preferred, ...urls.map((_, i) => i).filter(i => i !== this.preferred)];
    for (const i of order) {
      const url = urls[i]!.replace(/\/$/, '') + '/api/v1' + path;
      const headers: Record<string, string> = {Authorization: 'Bearer ' + this.enrollment.token, Accept: 'application/json'};
      if (body !== undefined) headers['Content-Type'] = 'application/json';
      let res;
      try {
        res = await this.native.request(url, method, JSON.stringify(headers), body === undefined ? null : JSON.stringify(body), this.enrollment.caSha256, TIMEOUT_MS);
      } catch (e) {
        // Network or TLS failure: another address may work; never weaken trust.
        lastError = e;
        continue;
      }
      this.preferred = i;
      if (res.status === 204) return undefined as T;
      let data: unknown;
      try { data = res.body ? JSON.parse(res.body) : undefined; } catch { data = undefined; }
      if (res.status >= 400) {
        const err = (data as {error?: {code?: string; message?: string}} | undefined)?.error;
        throw new DeviceError(err?.code ?? 'http_' + res.status, err?.message ?? 'request failed', res.status);
      }
      return data as T;
    }
    throw new DeviceError('unreachable', `Cannot reach the Stephania device (${String((lastError as Error)?.message ?? lastError)})`);
  }

  status = () => this.call<Status>('/status');
  start = () => this.call<{accepted: boolean}>('/stream/start', 'POST');
  stop = () => this.call<{accepted: boolean}>('/stream/stop', 'POST');
  scenes = () => this.call<StoredScene[]>('/scenes');
  transition = (scene_id: string, revision: number, fadeMs?: number) =>
    this.call<{scene_id: string; revision: number}>('/program/transition', 'POST',
      fadeMs ? {scene_id, revision, transition: {type: 'fade', duration_ms: fadeMs}} : {scene_id, revision});
  audio = () => this.call<{stored: AudioInput[]; engine: unknown}>('/audio');
  setMuted = (id: AudioInput['id'], muted: boolean) => this.call<{stored: AudioInput[]; applied: boolean}>('/audio', 'PUT', {inputs: [{id, muted}]});
  // Phone camera session (docs/PHONE.md): non-trickle SDP offer/answer.
  cameraOffer = (sdp: string, tracks: Record<string, 'rear' | 'front'>, replace = false) =>
    this.call<{sdp: string}>('/cameras/session/offer', 'POST', replace ? {sdp, tracks, replace: true} : {sdp, tracks});
  cameraSession = () => this.call<{state: string; access_units?: number; dropped?: number}>('/cameras/session');
  cameraClose = () => this.call<void>('/cameras/session', 'DELETE');
}

/** "ab12…" → "AB12 CD34 …" for comparing fingerprints by eye. */
export function formatFingerprint(hex: string): string {
  return (hex.toUpperCase().match(/.{1,4}/g) ?? []).join(' ');
}
