// Pure scene-editing operations for the Studio canvas: move, resize with
// handles, aspect lock, snapping, z-order, add/remove, and an undo/redo
// history. Coordinates are canvas pixels (1920×1080), integers, always
// clamped to scene.schema.json bounds so every edit stays saveable.
export type Source = 'hdmi' | 'phone:rear' | 'phone:front';
export type Fit = 'stretch' | 'contain' | 'cover';
export type Crop = {left: number; top: number; right: number; bottom: number};
export type Template = {id: string; vars: Record<string, string>};
type Common = {id: string; x: number; y: number; width: number; height: number; visible: boolean; opacity?: number; rotation?: number};
/** v1 video layers have no `type`; v2 layers always do (docs/SCENE.md). */
export type VideoLayer = Common & {type?: 'video'; source: Source; fit?: Fit; crop?: Crop};
export type ImageLayer = Common & {type: 'image'; asset: string; fit?: Fit; template?: Template};
export type ColorLayer = Common & {type: 'color'; color: string};
export type Layer = VideoLayer | ImageLayer | ColorLayer;
export type Scene = {version: 1 | 2; name: string; canvas?: {background: string}; layers: Layer[]};

export const isVideo = (l: Layer): l is VideoLayer => l.type === undefined || l.type === 'video';
export const MAX_CROP_SUM = 9000;

/** Explicit v1 → v2 migration: every layer becomes a typed video layer. */
export function migrateToV2(s: Scene): Scene {
  if (s.version === 2) return s;
  return {version: 2, name: s.name, layers: s.layers.map(l => ({...(l as VideoLayer), type: 'video' as const}))};
}

export function layerLabel(l: Layer): string {
  return isVideo(l) ? l.source : l.type === 'image' ? (l.template ? 'テンプレート:' + l.template.id : '画像') : '色 ' + l.color;
}

export const CANVAS = {width: 1920, height: 1080} as const;
export const MAX_LAYERS = 16;
export const MIN_SIZE = 16;
export const SOURCES: Source[] = ['hdmi', 'phone:rear', 'phone:front'];

export type Handle = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';
export const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Snap `v` to the nearest multiple of `grid` when within `grid/2`. */
export function snap(v: number, grid: number): number {
  return grid > 1 ? Math.round(v / grid) * grid : Math.round(v);
}

export function moveLayer(l: Layer, dx: number, dy: number, grid = 1): Layer {
  const x = clamp(snap(l.x + dx, grid), 0, CANVAS.width - l.width);
  const y = clamp(snap(l.y + dy, grid), 0, CANVAS.height - l.height);
  return {...l, x, y};
}

/**
 * Resize from `start` by dragging `handle` by (dx, dy). The opposite edge or
 * corner stays fixed. With `lockAspect` the start aspect ratio is kept (the
 * dominant drag axis wins for corners; edges scale the other axis around
 * the centre line). Result is clamped to the canvas and MIN_SIZE.
 */
export function resizeLayer(start: Layer, handle: Handle, dx: number, dy: number, opts: {lockAspect?: boolean; grid?: number} = {}): Layer {
  const grid = opts.grid ?? 1;
  let left = start.x, top = start.y, right = start.x + start.width, bottom = start.y + start.height;
  if (handle.includes('w')) left = clamp(snap(left + dx, grid), 0, right - MIN_SIZE);
  if (handle.includes('e')) right = clamp(snap(right + dx, grid), left + MIN_SIZE, CANVAS.width);
  if (handle.includes('n')) top = clamp(snap(top + dy, grid), 0, bottom - MIN_SIZE);
  if (handle.includes('s')) bottom = clamp(snap(bottom + dy, grid), top + MIN_SIZE, CANVAS.height);
  if (opts.lockAspect) {
    const ratio = start.width / start.height;
    let w = right - left, h = bottom - top;
    const horizontal = handle === 'e' || handle === 'w';
    const vertical = handle === 'n' || handle === 's';
    if (horizontal || (!vertical && Math.abs(dx) >= Math.abs(dy) * ratio)) h = w / ratio;
    else w = h * ratio;
    // Fit inside the canvas from the fixed anchor, shrinking both sides.
    const anchorX = handle.includes('w') ? right : handle.includes('e') ? left : start.x + start.width / 2;
    const anchorY = handle.includes('n') ? bottom : handle.includes('s') ? top : start.y + start.height / 2;
    const roomX = handle.includes('w') ? anchorX : handle.includes('e') ? CANVAS.width - anchorX : 2 * Math.min(anchorX, CANVAS.width - anchorX);
    const roomY = handle.includes('n') ? anchorY : handle.includes('s') ? CANVAS.height - anchorY : 2 * Math.min(anchorY, CANVAS.height - anchorY);
    const scale = Math.min(1, roomX / w, roomY / h);
    w = Math.max(MIN_SIZE, Math.round(w * scale));
    h = Math.max(MIN_SIZE, Math.round(h * scale));
    left = handle.includes('w') ? anchorX - w : handle.includes('e') ? anchorX : Math.round(anchorX - w / 2);
    top = handle.includes('n') ? anchorY - h : handle.includes('s') ? anchorY : Math.round(anchorY - h / 2);
    right = left + w;
    bottom = top + h;
  }
  return normalize({...start, x: left, y: top, width: right - left, height: bottom - top});
}

/** Clamp a layer into schema bounds (integers, inside the canvas). */
export function normalize<T extends Layer>(l: T): T {
  const width = clamp(Math.round(l.width), 1, CANVAS.width);
  const height = clamp(Math.round(l.height), 1, CANVAS.height);
  const x = clamp(Math.round(l.x), 0, CANVAS.width - width);
  const y = clamp(Math.round(l.y), 0, CANVAS.height - height);
  return {...l, x, y, width, height};
}

const clampInt = (v: number, lo: number, hi: number) => clamp(Math.round(Number.isFinite(v) ? v : 0), lo, hi);

/** Clamp v2 properties into their contract ranges. */
function normalizeV2<T extends Layer>(l: T): T {
  const out = {...l};
  if (out.opacity !== undefined) out.opacity = clampInt(out.opacity, 0, 100);
  // Wrap into −180…179 so a spinner can turn past ±180.
  if (out.rotation !== undefined) out.rotation = ((clampInt(out.rotation, -100000, 100000) + 180) % 360 + 360) % 360 - 180;
  if (isVideo(out) && out.crop) {
    const c = {...out.crop};
    for (const k of ['left', 'top', 'right', 'bottom'] as const) c[k] = clampInt(c[k], 0, MAX_CROP_SUM);
    if (c.left + c.right > MAX_CROP_SUM) c.right = MAX_CROP_SUM - c.left;
    if (c.top + c.bottom > MAX_CROP_SUM) c.bottom = MAX_CROP_SUM - c.top;
    (out as VideoLayer).crop = c;
  }
  return out;
}

export function updateLayer(s: Scene, id: string, change: Partial<VideoLayer> | Partial<ImageLayer> | Partial<ColorLayer>): Scene {
  return {...s, layers: s.layers.map(l => (l.id === id ? normalizeV2(normalize({...l, ...change} as Layer)) : l))};
}

export type ZMove = 'front' | 'forward' | 'backward' | 'back';

/** Layers paint in array order; the last one is on top. */
export function reorder(s: Scene, id: string, move: ZMove): Scene {
  const i = s.layers.findIndex(l => l.id === id);
  if (i < 0) return s;
  const layers = s.layers.slice();
  const [layer] = layers.splice(i, 1);
  const to = move === 'front' ? layers.length : move === 'back' ? 0
    : move === 'forward' ? Math.min(layers.length, i + 1) : Math.max(0, i - 1);
  layers.splice(to, 0, layer!);
  return {...s, layers};
}

function freeId(s: Scene, base: string): string {
  let id = base;
  for (let n = 2; s.layers.some(l => l.id === id); n++) id = `${base}-${n}`;
  return id;
}

function push(s: Scene, layer: Layer): {scene: Scene; id: string} | undefined {
  if (s.layers.length >= MAX_LAYERS) return undefined;
  return {scene: {...s, layers: [...s.layers, normalize(layer)]}, id: layer.id};
}

export function addLayer(s: Scene, source: Source): {scene: Scene; id: string} | undefined {
  const id = freeId(s, source.replace(':', '-'));
  const offset = (s.layers.length * 40) % 400;
  const base = {id, source, x: 1440 - offset, y: offset, width: 480, height: 270, visible: true};
  return push(s, s.version === 2 ? {...base, type: 'video'} : base);
}

/** Colour layer (scene v2 only). */
export function addColorLayer(s: Scene, color: string): {scene: Scene; id: string} | undefined {
  if (s.version !== 2) return undefined;
  return push(s, {id: freeId(s, 'color'), type: 'color', color, x: 0, y: 900, width: 1920, height: 180, visible: true});
}

/** Image layer sized to the image (at most 960 px wide), scene v2 only. */
export function addImageLayer(s: Scene, asset: string, width: number, height: number, opts: {template?: Template; x?: number; y?: number; base?: string} = {}): {scene: Scene; id: string} | undefined {
  if (s.version !== 2) return undefined;
  const scale = Math.min(1, 960 / width, 540 / height);
  const w = Math.max(1, Math.round(width * scale)), h = Math.max(1, Math.round(height * scale));
  const layer: ImageLayer = {id: freeId(s, opts.base ?? 'image'), type: 'image', asset, x: opts.x ?? 80, y: opts.y ?? 80, width: w, height: h, visible: true,
    ...(opts.template ? {template: opts.template} : {})};
  return push(s, layer);
}

export function removeLayer(s: Scene, id: string): Scene {
  return {...s, layers: s.layers.filter(l => l.id !== id)};
}

/** Bounded undo/redo history of whole scene documents. */
export type History = {past: Scene[]; present: Scene; future: Scene[]};
export const HISTORY_LIMIT = 100;

export function historyOf(present: Scene): History {
  return {past: [], present, future: []};
}

/** Record a new state. `coalesce` replaces the present instead (continuous drags). */
export function commit(h: History, next: Scene, coalesce = false): History {
  if (next === h.present) return h;
  if (coalesce) return {...h, present: next, future: []};
  const past = [...h.past, h.present].slice(-HISTORY_LIMIT);
  return {past, present: next, future: []};
}

export function undo(h: History): History {
  const prev = h.past[h.past.length - 1];
  if (!prev) return h;
  return {past: h.past.slice(0, -1), present: prev, future: [h.present, ...h.future]};
}

export function redo(h: History): History {
  const [next, ...future] = h.future;
  if (!next) return h;
  return {past: [...h.past, h.present], present: next, future};
}
