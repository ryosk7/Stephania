// Typed client for contracts/openapi.yaml. Types in schema.d.ts are generated
// (`npm run gen:api`); CI fails if they drift from the contract.
import type {components, paths} from './schema';

export type StoredScene = components['schemas']['StoredScene'];
export type SceneDocument = StoredScene['document'];
export type EngineStatus = paths['/status']['get']['responses']['200']['content']['application/json'];
export type Revision = paths['/scenes/{id}/revisions']['get']['responses']['200']['content']['application/json'][number];

/** Error envelope `{error:{code,message,request_id}}` as a thrown value. */
export class ApiError extends Error {
  constructor(readonly status: number, readonly code: string, message: string, readonly requestId?: string) {
    super(`${code}: ${message}`);
  }
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

export function createClient(token: () => string, fetcher: Fetch = (i, n) => fetch(i, n)) {
  async function call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
    const res = await fetcher('/api/v1' + path, {
      method,
      headers: {Authorization: 'Bearer ' + token(), ...(body === undefined ? {} : {'Content-Type': 'application/json'})},
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
  const id = (s: string) => encodeURIComponent(s);
  return {
    status: () => call<EngineStatus>('/status'),
    start: () => call<Record<string, unknown>>('/stream/start', 'POST'),
    stop: () => call<Record<string, unknown>>('/stream/stop', 'POST'),
    scenes: () => call<StoredScene[]>('/scenes'),
    scene: (s: string) => call<StoredScene>(`/scenes/${id(s)}`),
    saveScene: (s: string, revision: number, document: SceneDocument) => call<StoredScene>(`/scenes/${id(s)}`, 'PUT', {revision, document}),
    deleteScene: (s: string, revision: number) => call<void>(`/scenes/${id(s)}?revision=${revision}`, 'DELETE'),
    revisions: (s: string) => call<Revision[]>(`/scenes/${id(s)}/revisions`),
    transition: (scene_id: string, revision: number) => call<unknown>('/program/transition', 'POST', {scene_id, revision}),
  };
}
