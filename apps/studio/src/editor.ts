// Pure scene-editing operations for the Studio canvas: move, resize with
// handles, aspect lock, snapping, z-order, add/remove, and an undo/redo
// history. Coordinates are canvas pixels (1920×1080), integers, always
// clamped to scene.schema.json bounds so every edit stays saveable.
export type Source = 'hdmi' | 'phone:rear' | 'phone:front';
export type Layer = {id: string; source: Source; x: number; y: number; width: number; height: number; visible: boolean};
export type Scene = {version: 1; name: string; layers: Layer[]};

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
export function normalize(l: Layer): Layer {
  const width = clamp(Math.round(l.width), 1, CANVAS.width);
  const height = clamp(Math.round(l.height), 1, CANVAS.height);
  const x = clamp(Math.round(l.x), 0, CANVAS.width - width);
  const y = clamp(Math.round(l.y), 0, CANVAS.height - height);
  return {...l, x, y, width, height};
}

export function updateLayer(s: Scene, id: string, change: Partial<Layer>): Scene {
  return {...s, layers: s.layers.map(l => (l.id === id ? normalize({...l, ...change}) : l))};
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

export function addLayer(s: Scene, source: Source): {scene: Scene; id: string} | undefined {
  if (s.layers.length >= MAX_LAYERS) return undefined;
  const base = source.replace(':', '-');
  let id = base;
  for (let n = 2; s.layers.some(l => l.id === id); n++) id = `${base}-${n}`;
  const offset = (s.layers.length * 40) % 400;
  const layer = normalize({id, source, x: 1440 - offset, y: offset, width: 480, height: 270, visible: true});
  return {scene: {...s, layers: [...s.layers, layer]}, id};
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
