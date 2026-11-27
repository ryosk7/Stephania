// Typed client for contracts/openapi.yaml. Types in schema.d.ts are generated
// (`npm run gen:api`); CI fails if they drift from the contract.
import type {components, paths} from './schema';

export type StoredScene = components['schemas']['StoredScene'];
export type SceneDocument = StoredScene['document'];
export type EngineStatus = paths['/status']['get']['responses']['200']['content']['application/json'];
export type AudioInput = components['schemas']['AudioInput'];
export type AudioOverview = paths['/audio']['get']['responses']['200']['content']['application/json'];
export type AudioUpdate = paths['/audio']['put']['requestBody']['content']['application/json'];
export type Revision = paths['/scenes/{id}/revisions']['get']['responses']['200']['content']['application/json'][number];
export type User = components['schemas']['User'];
export type Role = components['schemas']['Role'];
export type Me = paths['/auth/me']['get']['responses']['200']['content']['application/json'];
export type Login = paths['/auth/session']['post']['responses']['201']['content']['application/json'];
export type SceneExport = components['schemas']['SceneExport'];
export type Asset = components['schemas']['Asset'];
export type OverlayTemplate = components['schemas']['OverlayTemplate'];
export type Capabilities = paths['/capabilities']['get']['responses']['200']['content']['application/json'];

/**
 * How requests authenticate: a bearer token (owner/bootstrap or enrolled
 * device token) or the Studio session cookie, which the browser sends by
 * itself; cookie writes carry the CSRF token.
 */
export type Credentials = {token?: string; csrf?: string};

/** Error envelope `{error:{code,message,request_id}}` as a thrown value. */
export class ApiError extends Error {
  constructor(readonly status: number, readonly code: string, message: string, readonly requestId?: string) {
    super(`${code}: ${message}`);
  }
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

export function authHeaders(c: Credentials, method = 'GET'): Record<string, string> {
  if (c.token) return {Authorization: 'Bearer ' + c.token};
  return c.csrf && method !== 'GET' && method !== 'HEAD' ? {'X-CSRF-Token': c.csrf} : {};
}

export function createClient(credentials: () => Credentials, fetcher: Fetch = (i, n) => fetch(i, n)) {
  async function call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
    const res = await fetcher('/api/v1' + path, {
      method,
      credentials: 'same-origin',
      headers: {...authHeaders(credentials(), method), ...(body === undefined ? {} : {'Content-Type': 'application/json'})},
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (res.status === 204) return undefined as T;
    const data: unknown = await res.json().catch(() => undefined);
    if (!res.ok) {
      const e = (data as {error?: {code?: unknown; message?: unknown; request_id?: unknown}} | undefined)?.error;
      if (e && typeof e.code === 'string') {
        throw new ApiError(res.status, e.code, String(e.message ?? ''), typeof e.request_id === 'string' ? e.request_id : undefined);
      }
      throw new ApiError(res.status, 'http_' + res.status, res.statusText || 'request failed');
    }
    return data as T;
  }
  async function upload(blob: Blob, name: string): Promise<Asset> {
    const c = credentials();
    const res = await fetcher('/api/v1/assets?name=' + encodeURIComponent(name), {
      method: 'POST', credentials: 'same-origin', headers: {...authHeaders(c, 'POST'), 'Content-Type': 'image/png'}, body: blob,
    });
    const data: unknown = await res.json().catch(() => undefined);
    if (!res.ok) {
      const e = (data as {error?: {code?: string; message?: string; request_id?: string}} | undefined)?.error;
      throw new ApiError(res.status, e?.code ?? 'http_' + res.status, e?.message ?? res.statusText, e?.request_id);
    }
    return data as Asset;
  }
  const id = (s: string) => encodeURIComponent(s);
  return {
    login: (username: string, password: string) => call<Login>('/auth/session', 'POST', {username, password}),
    logout: () => call<void>('/auth/session', 'DELETE'),
    me: () => call<Me>('/auth/me'),
    changePassword: (current_password: string, new_password: string) => call<void>('/auth/password', 'PUT', {current_password, new_password}),
    users: () => call<User[]>('/users'),
    createUser: (username: string, password: string, role: Role) => call<User>('/users', 'POST', {username, password, role}),
    updateUser: (uid: number, change: {role?: Role; disabled?: boolean; password?: string}) => call<User>(`/users/${uid}`, 'PATCH', change),
    deleteUser: (uid: number) => call<void>(`/users/${uid}`, 'DELETE'),
    capabilities: () => call<Capabilities>('/capabilities'),
    status: () => call<EngineStatus>('/status'),
    start: () => call<Record<string, unknown>>('/stream/start', 'POST'),
    stop: () => call<Record<string, unknown>>('/stream/stop', 'POST'),
    scenes: () => call<StoredScene[]>('/scenes'),
    scene: (s: string) => call<StoredScene>(`/scenes/${id(s)}`),
    saveScene: (s: string, revision: number, document: SceneDocument) => call<StoredScene>(`/scenes/${id(s)}`, 'PUT', {revision, document}),
    deleteScene: (s: string, revision: number) => call<void>(`/scenes/${id(s)}?revision=${revision}`, 'DELETE'),
    revisions: (s: string) => call<Revision[]>(`/scenes/${id(s)}/revisions`),
    createScene: (name: string, document?: SceneDocument) => call<StoredScene>('/scenes', 'POST', document ? {name, document} : {name}),
    duplicateScene: (s: string) => call<StoredScene>(`/scenes/${id(s)}/duplicate`, 'POST'),
    exportScene: (s: string) => call<SceneExport>(`/scenes/${id(s)}/export`),
    importScene: (e: SceneExport) => call<StoredScene>('/scenes/import', 'POST', e),
    transition: (scene_id: string, revision: number, transition?: {type: 'cut'} | {type: 'fade'; duration_ms: number}) =>
      call<unknown>('/program/transition', 'POST', transition ? {scene_id, revision, transition} : {scene_id, revision}),
    assets: () => call<Asset[]>('/assets'),
    templates: () => call<OverlayTemplate[]>('/templates'),
    saveTemplate: (t: OverlayTemplate) => { const {id: tid, ...body} = t; return call<OverlayTemplate>(`/templates/${id(tid)}`, 'PUT', body); },
    deleteTemplate: (t: string) => call<void>(`/templates/${id(t)}`, 'DELETE'),
    uploadAsset: upload,
    deleteAsset: (a: string) => call<void>(`/assets/${id(a)}`, 'DELETE'),
    /** Asset bytes as a Blob (works with bearer tokens, unlike <img src>). */
    assetBlob: async (a: string) => {
      const res = await fetcher(`/api/v1/assets/${id(a)}`, {credentials: 'same-origin', headers: authHeaders(credentials())});
      if (!res.ok) throw new ApiError(res.status, 'http_' + res.status, 'asset unavailable');
      return res.blob();
    },
    audio: () => call<AudioOverview>('/audio'),
    saveAudio: (update: AudioUpdate) => call<AudioOverview & {applied: boolean}>('/audio', 'PUT', update),
  };
}
