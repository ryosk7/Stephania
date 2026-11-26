// Draft scene v1 validation for the editor. Mirrors src/scene.rs and
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

export function validateScene(doc: unknown): Violation | null {
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
  if (/"(x|y|width|height)"\s*:\s*-?\d+(\.\d+|[eE][+-]?\d+)/.test(text)) return {doc, violation: {path: '/layers', reason: 'not_integer'}};
  return {doc, violation: validateScene(doc)};
}
