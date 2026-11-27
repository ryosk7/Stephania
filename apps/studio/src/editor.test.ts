import {describe, expect, it} from 'vitest';
import {
  addLayer, CANVAS, commit, historyOf, MAX_LAYERS, MIN_SIZE, moveLayer, redo, removeLayer, reorder, resizeLayer, undo, updateLayer,
  type Layer, type Scene,
} from './editor';
import {validateScene} from './scene';

const layer = (over: Partial<Layer> = {}): Layer => ({id: 'a', source: 'hdmi', x: 100, y: 100, width: 400, height: 200, visible: true, ...over});
const scene = (...layers: Layer[]): Scene => ({version: 1, name: 'T', layers});

describe('move', () => {
  it('clamps inside the canvas and snaps to the grid', () => {
    expect(moveLayer(layer(), -500, -500)).toMatchObject({x: 0, y: 0});
    expect(moveLayer(layer(), 5000, 5000)).toMatchObject({x: CANVAS.width - 400, y: CANVAS.height - 200});
    expect(moveLayer(layer(), 13, 6, 8)).toMatchObject({x: 112, y: 104});
  });
});

describe('resize', () => {
  it('keeps the opposite corner fixed', () => {
    const r = resizeLayer(layer(), 'nw', -50, -20);
    expect(r).toMatchObject({x: 50, y: 80, width: 450, height: 220});
    expect(r.x + r.width).toBe(500);
    expect(r.y + r.height).toBe(300);
  });
  it('edges change one axis only', () => {
    expect(resizeLayer(layer(), 'e', 100, 999)).toMatchObject({x: 100, y: 100, width: 500, height: 200});
    expect(resizeLayer(layer(), 's', 999, 30)).toMatchObject({width: 400, height: 230});
  });
  it('never inverts and respects the minimum size', () => {
    expect(resizeLayer(layer(), 'se', -1000, -1000)).toMatchObject({x: 100, y: 100, width: MIN_SIZE, height: MIN_SIZE});
    expect(resizeLayer(layer(), 'nw', 1000, 1000)).toMatchObject({width: MIN_SIZE, height: MIN_SIZE, x: 500 - MIN_SIZE});
  });
  it('clamps to the canvas', () => {
    const r = resizeLayer(layer(), 'se', 5000, 5000);
    expect(r.x + r.width).toBe(CANVAS.width);
    expect(r.y + r.height).toBe(CANVAS.height);
  });
  it('locks the aspect ratio from corners and edges, staying inside the canvas', () => {
    const r = resizeLayer(layer(), 'se', 200, 0, {lockAspect: true});
    expect(r).toMatchObject({x: 100, y: 100, width: 600, height: 300});
    const e = resizeLayer(layer(), 'e', 100, 0, {lockAspect: true});
    expect(e.width / e.height).toBeCloseTo(2, 2);
    expect(e.y + e.height / 2).toBeCloseTo(200, 0);
    const big = resizeLayer(layer(), 'se', 5000, 5000, {lockAspect: true});
    expect(big.width / big.height).toBeCloseTo(2, 2);
    expect(big.x + big.width).toBeLessThanOrEqual(CANVAS.width);
    expect(big.y + big.height).toBeLessThanOrEqual(CANVAS.height);
  });
  it('always yields a schema-valid scene', () => {
    for (const h of ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'] as const) {
      for (const [dx, dy] of [[-3000, -3000], [3000, 3000], [17, -9]]) {
        for (const lockAspect of [false, true]) {
          const s = scene(resizeLayer(layer(), h, dx!, dy!, {lockAspect, grid: 8}));
          expect(validateScene(s), `${h} ${dx},${dy} ${lockAspect}`).toBeNull();
        }
      }
    }
  });
});

describe('layers', () => {
  const s = scene(layer({id: 'a'}), layer({id: 'b'}), layer({id: 'c'}));
  const order = (x: Scene) => x.layers.map(l => l.id).join('');
  it('reorders z', () => {
    expect(order(reorder(s, 'a', 'front'))).toBe('bca');
    expect(order(reorder(s, 'c', 'back'))).toBe('cab');
    expect(order(reorder(s, 'a', 'forward'))).toBe('bac');
    expect(order(reorder(s, 'a', 'backward'))).toBe('abc');
    expect(order(reorder(s, 'c', 'forward'))).toBe('abc');
  });
  it('adds unique ids up to the limit and removes', () => {
    const a = addLayer(scene(layer({id: 'phone-rear'})), 'phone:rear')!;
    expect(a.id).toBe('phone-rear-2');
    expect(validateScene(a.scene)).toBeNull();
    let full = scene();
    for (let i = 0; i < MAX_LAYERS; i++) full = addLayer(full, 'hdmi')!.scene;
    expect(addLayer(full, 'hdmi')).toBeUndefined();
    expect(order(removeLayer(s, 'b'))).toBe('ac');
  });
  it('numeric edits are normalised', () => {
    expect(updateLayer(s, 'a', {x: 5000, width: 99999}).layers[0]).toMatchObject({x: 0, width: CANVAS.width});
  });
});

describe('history', () => {
  it('undoes and redoes, a new edit clears redo, drags coalesce', () => {
    const s0 = scene(layer());
    const s1 = updateLayer(s0, 'a', {x: 1});
    const s2 = updateLayer(s1, 'a', {x: 2});
    let h = commit(commit(historyOf(s0), s1), s2);
    h = undo(h);
    expect(h.present).toBe(s1);
    h = redo(h);
    expect(h.present).toBe(s2);
    h = commit(undo(h), s0);
    expect(h.future).toEqual([]);
    // One drag = one undo step.
    let d = commit(historyOf(s0), s1);
    d = commit(d, s2, true);
    expect(undo(d).present).toBe(s0);
    expect(undo(historyOf(s0)).present).toBe(s0);
  });
});
