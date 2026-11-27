// Scene validation (contract versions 1 and 2, docs/SCENE.md) for the editor. Mirrors src/scene.rs and
// services/control/lib/scene_validator.rb; all three run the shared
// contracts/fixtures/scenes suite. The server and engine remain authoritative.
export const CANVAS = {width: 1920, height: 1080} as const;
export const SOURCES_V1 = ['hdmi', 'phone:rear', 'phone:front'] as const;
const ROOT_KEYS = ['layers', 'name', 'version'];
const LAYER_KEYS = ['height', 'id', 'source', 'visible', 'width', 'x', 'y'];
const ID = /^[a-zA-Z0-9_-]{1,64}$/;

export type Violation = {path: string; reason: string};

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const sameKeys = (o: Record<string, unknown>, keys: string[]) => {
  const k = Object.keys(o).sort();
  return k.length === keys.length && k.every((x, i) => x === keys[i]);
};
// JSON.parse turns 1.0 into 1, so float detection relies on the raw text;
// callers validating raw JSON should use validateSceneText.
const isInt = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v);

export const FITS = ['stretch', 'contain', 'cover'] as const;
const BY_TYPE: Record<string, [string[], string[]]> = {
  video: [['source'], ['opacity', 'rotation', 'fit', 'crop']],
  image: [['asset'], ['opacity', 'rotation', 'fit', 'template']],
  color: [['color'], ['opacity', 'rotation']],
};
const COMMON_V2 = ['id', 'type', 'x', 'y', 'width', 'height', 'visible'];
const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;
const ASSET = /^[0-9a-f]{16}$/;
const TEMPLATE_ID = /^[a-z0-9-]{1,32}$/;
const TEMPLATE_KEY = /^[a-z][a-z0-9_.]{0,31}$/;
const MAX_CROP_SUM = 9000;
const keysOk = (o: Record<string, unknown>, required: string[], optional: string[]) =>
  required.every(k => k in o) && Object.keys(o).every(k => required.includes(k) || optional.includes(k));
const chars = (s: string) => [...s].length;

function templateOk(t: unknown): boolean {
  if (!isObject(t) || !sameKeys(t, ['id', 'vars']) || typeof t.id !== 'string' || !TEMPLATE_ID.test(t.id) || !isObject(t.vars)) return false;
  const vars = Object.entries(t.vars);
  return vars.length <= 16 && vars.every(([k, v]) => TEMPLATE_KEY.test(k) && typeof v === 'string' && chars(v) <= 200);
}

function validateV2(doc: Record<string, unknown>): Violation | null {
  if (!keysOk(doc, ['version', 'name', 'layers'], ['canvas'])) return {path: '', reason: 'unexpected_or_missing_property'};
  const name = doc.name;
  if (typeof name !== 'string' || chars(name) < 1 || chars(name) > 100) return {path: '/name', reason: 'invalid_name'};
  if ('canvas' in doc) {
    const c = doc.canvas;
    if (!isObject(c) || !sameKeys(c, ['background']) || typeof c.background !== 'string' || !HEX_COLOR.test(c.background)) return {path: '/canvas', reason: 'invalid_canvas'};
  }
  const layers = doc.layers;
  if (!Array.isArray(layers)) return {path: '/layers', reason: 'not_array'};
  if (layers.length > 16) return {path: '/layers', reason: 'too_many_layers'};
  const seen = new Set<string>();
  for (const [i, layer] of layers.entries()) {
    const path = `/layers/${i}`;
    if (!isObject(layer)) return {path, reason: 'not_object'};
    const kind = layer.type;
    if (typeof kind !== 'string' || !(kind in BY_TYPE)) return {path: path + '/type', reason: 'unknown_type'};
    const [required, optional] = BY_TYPE[kind]!;
    if (!keysOk(layer, [...COMMON_V2, ...required], optional)) return {path, reason: 'unexpected_or_missing_property'};
    if (typeof layer.id !== 'string' || !ID.test(layer.id)) return {path: path + '/id', reason: 'invalid_id'};
    if (seen.has(layer.id)) return {path: path + '/id', reason: 'duplicate_id'};
    seen.add(layer.id);
    if (typeof layer.visible !== 'boolean') return {path: path + '/visible', reason: 'not_boolean'};
    for (const k of ['x', 'y', 'width', 'height']) if (!isInt(layer[k])) return {path: `${path}/${k}`, reason: 'not_integer'};
    const [x, y, w, h] = [layer.x, layer.y, layer.width, layer.height] as number[];
    if (x < 0 || y < 0 || w <= 0 || h <= 0) return {path, reason: 'negative_or_empty_geometry'};
    if (x + w > CANVAS.width || y + h > CANVAS.height) return {path, reason: 'outside_canvas'};
    for (const [k, lo, hi, reason] of [['opacity', 0, 100, 'invalid_opacity'], ['rotation', -180, 180, 'invalid_rotation']] as const) {
      if (!(k in layer)) continue;
      if (!isInt(layer[k])) return {path: `${path}/${k}`, reason: 'not_integer'};
      if ((layer[k] as number) < lo || (layer[k] as number) > hi) return {path: `${path}/${k}`, reason};
    }
    if ('fit' in layer && !(FITS as readonly unknown[]).includes(layer.fit)) return {path: path + '/fit', reason: 'invalid_fit'};
    if (kind === 'video') {
      if (!(SOURCES_V1 as readonly unknown[]).includes(layer.source)) return {path: path + '/source', reason: 'unknown_source'};
      if ('crop' in layer) {
        const c = layer.crop;
        const ok = isObject(c) && sameKeys(c, ['bottom', 'left', 'right', 'top'])
          && Object.values(c).every(v => isInt(v) && v >= 0 && v <= MAX_CROP_SUM)
          && (c.left as number) + (c.right as number) <= MAX_CROP_SUM && (c.top as number) + (c.bottom as number) <= MAX_CROP_SUM;
        if (!ok) return {path: path + '/crop', reason: 'invalid_crop'};
      }
    } else if (kind === 'image') {
      if (typeof layer.asset !== 'string' || !ASSET.test(layer.asset)) return {path: path + '/asset', reason: 'invalid_asset'};
      if ('template' in layer && !templateOk(layer.template)) return {path: path + '/template', reason: 'invalid_template'};
    } else if (typeof layer.color !== 'string' || !HEX_COLOR.test(layer.color)) {
      return {path: path + '/color', reason: 'invalid_color'};
    }
  }
  return null;
}

export function validateScene(doc: unknown): Violation | null {
  if (isObject(doc) && doc.version === 2) return validateV2(doc);
  if (!isObject(doc)) return {path: '', reason: 'not_object'};
  if (!sameKeys(doc, ROOT_KEYS)) return {path: '', reason: 'unexpected_or_missing_property'};
  if (doc.version !== 1) return {path: '/version', reason: 'unsupported_version'};
  const name = doc.name;
  if (typeof name !== 'string' || [...name].length < 1 || [...name].length > 100) return {path: '/name', reason: 'invalid_name'};
  const layers = doc.layers;
  if (!Array.isArray(layers)) return {path: '/layers', reason: 'not_array'};
  if (layers.length > 16) return {path: '/layers', reason: 'too_many_layers'};
  const seen = new Set<string>();
  for (const [i, layer] of layers.entries()) {
    const path = `/layers/${i}`;
    if (!isObject(layer)) return {path, reason: 'not_object'};
    if (!sameKeys(layer, LAYER_KEYS)) return {path, reason: 'unexpected_or_missing_property'};
    if (typeof layer.id !== 'string' || !ID.test(layer.id)) return {path: path + '/id', reason: 'invalid_id'};
    if (seen.has(layer.id)) return {path: path + '/id', reason: 'duplicate_id'};
    seen.add(layer.id);
    if (!(SOURCES_V1 as readonly unknown[]).includes(layer.source)) return {path: path + '/source', reason: 'unknown_source'};
    if (typeof layer.visible !== 'boolean') return {path: path + '/visible', reason: 'not_boolean'};
    for (const k of ['x', 'y', 'width', 'height']) if (!isInt(layer[k])) return {path: `${path}/${k}`, reason: 'not_integer'};
    const [x, y, w, h] = [layer.x, layer.y, layer.width, layer.height] as number[];
    if (x < 0 || y < 0 || w <= 0 || h <= 0) return {path, reason: 'negative_or_empty_geometry'};
    if (x + w > CANVAS.width || y + h > CANVAS.height) return {path, reason: 'outside_canvas'};
  }
  return null;
}

/** Validate raw JSON text, rejecting fractional-looking geometry like `1.0`. */
export function validateSceneText(text: string): {doc: unknown; violation: Violation | null} {
  let doc: unknown;
  try { doc = JSON.parse(text); } catch { return {doc: undefined, violation: {path: '', reason: 'invalid_json'}}; }
  if (/"(x|y|width|height|opacity|rotation|left|top|right|bottom)"\s*:\s*-?\d+(\.\d+|[eE][+-]?\d+)/.test(text)) return {doc, violation: {path: '/layers', reason: 'not_integer'}};
  return {doc, violation: validateScene(doc)};
}
