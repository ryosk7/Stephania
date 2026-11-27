// Studio workspace: scene list (create/duplicate/delete/import/export),
// canvas with drag and resize handles, layer stack with z-order and
// visibility, numeric properties, undo/redo, draft history, JSON editing and
// the Program transition. Saving a draft never changes Program.
import React, {useCallback, useEffect, useMemo, useReducer, useRef, useState} from 'react';
import {ApiError, type createClient, type Revision, type SceneExport, type StoredScene} from './api/client';
import {
  addColorLayer, addImageLayer, addLayer, CANVAS, commit, HANDLES, historyOf, layerLabel, migrateToV2, moveLayer, redo, removeLayer, reorder,
  resizeLayer, SOURCES, undo, updateLayer, type Handle, type History, type Layer, type Scene, type Source, type ZMove,
} from './editor';
import {AssetLibrary, layerStyle, reference, TemplatePanel, useAssetUrls, V2Props} from './overlays';
import type {LiveState} from './live';
import {validateSceneText} from './scene';

type Api = ReturnType<typeof createClient>;

export const DEFAULT_SCENE: Scene = {version: 2, name: 'LT', layers: [
  {id: 'hdmi', type: 'video', source: 'hdmi', x: 0, y: 0, width: 1440, height: 1080, visible: true},
  {id: 'rear', type: 'video', source: 'phone:rear', x: 1440, y: 0, width: 480, height: 540, visible: true},
  {id: 'front', type: 'video', source: 'phone:front', x: 1440, y: 540, width: 480, height: 540, visible: true},
]};
const SOURCE_NAMES: Record<Source, string> = {hdmi: 'HDMI', 'phone:rear': 'Phone (rear)', 'phone:front': 'Phone (front)'};

type Action =
  | {type: 'load'; scene: Scene}
  | {type: 'edit'; scene: Scene; coalesce?: boolean}
  | {type: 'undo'} | {type: 'redo'};

function reducer(h: History, a: Action): History {
  switch (a.type) {
    case 'load': return historyOf(a.scene);
    case 'edit': return commit(h, a.scene, a.coalesce);
    case 'undo': return undo(h);
    case 'redo': return redo(h);
  }
}

const sameDoc = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export function Studio({api, live, canEdit, onMessage}: {api: Api; live: LiveState; canEdit: boolean; onMessage: (m: string) => void}) {
  const [scenes, setScenes] = useState<StoredScene[]>([]);
  const [current, setCurrent] = useState('lt');
  const [revision, setRevision] = useState(0);
  const [saved, setSaved] = useState<Scene | null>(null);
  const [h, dispatch] = useReducer(reducer, DEFAULT_SCENE, historyOf);
  const [selected, setSelected] = useState<string>('hdmi');
  const [lockAspect, setLockAspect] = useState(false);
  const [grid, setGrid] = useState(8);
  const [code, setCode] = useState('');
  const [revisions, setRevisions] = useState<Revision[]>([]);
  const [newName, setNewName] = useState('');
  const [addSource, setAddSource] = useState<Source>('phone:rear');
  const [transition, setTransition] = useState<'cut' | 'fade'>('cut');
  const [fadeMs, setFadeMs] = useState(500);
  const scene = h.present;
  const v2 = scene.version === 2;
  const assetUrls = useAssetUrls(api, scene.layers.flatMap(l => (l.type === 'image' ? [l.asset] : [])));
  const layer = scene.layers.find(l => l.id === selected);
  const dirty = saved ? !sameDoc(saved, scene) : revision === 0;
  const remote = live.scenes[current];
  const stale = remote !== undefined && remote !== revision;

  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }

  const open = useCallback(async (s: StoredScene) => {
    setCurrent(s.id);
    setRevision(s.revision);
    setSaved(s.document as Scene);
    dispatch({type: 'load', scene: s.document as Scene});
    setSelected((s.document as Scene).layers[0]?.id ?? '');
    setRevisions(await api.revisions(s.id));
  }, [api]);

  async function refresh() { setScenes(await api.scenes()); }

  async function reload() {
    await refresh();
    try {
      await open(await api.scene(current));
      onMessage('Loaded');
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) onMessage('No saved draft');
      else throw e;
    }
  }

  useEffect(() => {
    void run(async () => {
      const list = await api.scenes();
      setScenes(list);
      const first = list.find(s => s.id === 'lt') ?? list[0];
      if (first) await open(first);
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const edit = useCallback((next: Scene, coalesce = false) => { if (canEdit) dispatch({type: 'edit', scene: next, coalesce}); }, [canEdit]);

  // Keyboard: undo/redo, nudge, delete. Ignored while typing in a field.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return;
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === 'z') { e.preventDefault(); dispatch({type: e.shiftKey ? 'redo' : 'undo'}); return; }
      if (mod && e.key.toLowerCase() === 'y') { e.preventDefault(); dispatch({type: 'redo'}); return; }
      if (!canEdit || !layer) return;
      const step = e.shiftKey ? 10 * Math.max(1, grid) : Math.max(1, grid);
      const d = {ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step]}[e.key];
      if (d) { e.preventDefault(); edit(updateLayer(scene, layer.id, moveLayer(layer, d[0]!, d[1]!, 1))); }
      if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); edit(removeLayer(scene, layer.id)); }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [scene, layer, grid, canEdit, edit]);

  // Pointer drag (move) and handle drag (resize). One gesture = one undo step.
  const canvasRef = useRef<HTMLDivElement>(null);
  function startDrag(e: React.PointerEvent, l: Layer, handle?: Handle) {
    e.stopPropagation();
    setSelected(l.id);
    if (!canEdit) return;
    e.preventDefault();
    const box = canvasRef.current!.getBoundingClientRect();
    const sx = e.clientX, sy = e.clientY, base = scene;
    let first = true;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    target.onpointermove = ev => {
      const dx = (ev.clientX - sx) * CANVAS.width / box.width;
      const dy = (ev.clientY - sy) * CANVAS.height / box.height;
      const next = handle ? resizeLayer(l, handle, dx, dy, {lockAspect: lockAspect || ev.shiftKey, grid}) : moveLayer(l, dx, dy, grid);
      edit(updateLayer(base, l.id, next), !first);
      first = false;
    };
    target.onlostpointercapture = () => { target.onpointermove = null; };
  }

  async function save() {
    const d = await api.saveScene(current, revision, scene as never);
    setRevision(d.revision);
    setSaved(d.document as Scene);
    setRevisions(await api.revisions(current));
    await refresh();
    onMessage('Draft saved');
  }

  async function createScene() {
    const name = newName.trim();
    if (!name) { onMessage('Enter a scene name'); return; }
    const s = await api.createScene(name);
    setNewName('');
    await refresh();
    await open(s);
    onMessage(`Created scene "${name}"`);
  }

  async function duplicate() {
    const s = await api.duplicateScene(current);
    await refresh();
    await open(s);
    onMessage(`Duplicated (${s.id})`);
  }

  async function remove() {
    if (revision < 1) return;
    await api.deleteScene(current, revision);
    const list = await api.scenes();
    setScenes(list);
    if (list[0]) await open(list[0]);
    else { setCurrent('lt'); setRevision(0); setSaved(null); setRevisions([]); dispatch({type: 'load', scene: DEFAULT_SCENE}); }
    onMessage('Scene deleted');
  }

  async function exportScene() {
    const e = await api.exportScene(current);
    const url = URL.createObjectURL(new Blob([JSON.stringify(e, null, 2)], {type: 'application/json'}));
    const a = document.createElement('a');
    a.href = url;
    a.download = `${current}.t3box-scene.json`;
    a.click();
    URL.revokeObjectURL(url);
    onMessage('Exported');
  }

  async function importFile(file: File) {
    let parsed: unknown;
    try { parsed = JSON.parse(await file.text()); } catch { onMessage('Not valid JSON'); return; }
    const s = await api.importScene(parsed as SceneExport);
    await refresh();
    await open(s);
    onMessage(`Imported (${s.id})`);
  }

  const order = useMemo(() => scene.layers.map((l, i) => ({l, i})).reverse(), [scene.layers]);

  return (
    <div className="studio" data-testid="studio">
      <section className="scenes" aria-label="scenes">
        <h2>Scenes</h2>
        <ul>
          {scenes.map(s => (
            <li key={s.id}><button aria-pressed={s.id === current} onClick={() => run(async () => { await open(s); onMessage('Loaded'); })}>
              {(s.document as Scene).name} <small>{s.id} r{s.revision}</small></button></li>
          ))}
          {!scenes.some(s => s.id === current) && <li><button aria-pressed>{scene.name} <small>unsaved</small></button></li>}
        </ul>
        {canEdit && <>
          <label>New scene <input value={newName} onChange={e => setNewName(e.target.value)} maxLength={100}/></label>
          <button onClick={() => run(createScene)}>Create</button>
          <button disabled={revision < 1} onClick={() => run(duplicate)}>Duplicate</button>
          <button disabled={revision < 1} onClick={() => run(remove)}>Delete</button>
          <button disabled={revision < 1} onClick={() => run(exportScene)}>Export</button>
          <label className="file">Import <input type="file" accept="application/json,.json" onChange={e => { const f = e.target.files?.[0]; e.target.value = ''; if (f) void run(() => importFile(f)); }}/></label>
        </>}
      </section>

      <div className="workspace">
        {stale && <p data-testid="remote-update">r{remote} was saved elsewhere. Reload before editing (saving now would conflict).</p>}
        <nav className="toolbar">
          <button disabled={!h.past.length || !canEdit} onClick={() => dispatch({type: 'undo'})} title="Ctrl+Z">Undo</button>
          <button disabled={!h.future.length || !canEdit} onClick={() => dispatch({type: 'redo'})} title="Ctrl+Shift+Z">Redo</button>
          <button onClick={() => run(reload)}>Reload</button>
          {canEdit && <button onClick={() => run(save)}>Save draft</button>}
          {canEdit && <button onClick={() => setCode(JSON.stringify(scene, null, 2))}>Edit JSON</button>}
          <label>Transition <select aria-label="transition" value={transition} onChange={e => setTransition(e.target.value as 'cut' | 'fade')}>
            <option value="cut">Cut</option><option value="fade">Fade</option></select></label>
          {transition === 'fade' && <label>Duration <input type="number" aria-label="fade ms" min={1} max={5000} step={100} value={fadeMs} onChange={e => setFadeMs(Number(e.target.value))}/>ms</label>}
          <button disabled={revision < 1} onClick={() => run(async () => {
            const t = transition === 'fade' ? {type: 'fade' as const, duration_ms: Math.min(5000, Math.max(1, Math.round(fadeMs)))} : undefined;
            const r = await api.transition(current, revision, t) as {engine?: {first_frame_index?: number}};
            onMessage('Program applied at frame ' + (r.engine?.first_frame_index ?? '?'));
          })}>Apply to Program</button>
          <span data-testid="dirty">{dirty ? 'unsaved changes' : 'saved'}</span>
        </nav>

        {!v2 && <p data-testid="v1-banner">This scene uses the legacy format (v1). Convert it to v2 to use images, colours, opacity, rotation and crop.
          {canEdit && <button onClick={() => { edit(migrateToV2(scene)); onMessage('Converted to v2 (save to keep it)'); }}>Convert to v2</button>}</p>}
        <h2>Preview layout (draft) — {scene.name}</h2>
        <div className="canvas" ref={canvasRef} onPointerDown={() => setSelected('')} style={{background: scene.canvas?.background}}>
          {scene.layers.map(l => l.visible && (
            <div key={l.id} data-testid={'layer-' + l.id} className={'layer ' + (l.type ?? 'video') + (selected === l.id ? ' selected' : '')}
              style={layerStyle(l)}
              onPointerDown={e => startDrag(e, l)}>
              {l.type === 'image' ? (assetUrls[l.asset]
                ? <img src={assetUrls[l.asset]} alt="" draggable={false} style={{objectFit: l.fit === 'cover' ? 'cover' : l.fit === 'contain' ? 'contain' : 'fill'}}/>
                : 'image') : l.type === 'color' ? null : layerLabel(l)}
              {canEdit && selected === l.id && HANDLES.map(hd => (
                <span key={hd} className={'handle ' + hd} data-handle={hd} aria-label={'resize ' + hd} onPointerDown={e => startDrag(e, l, hd)}/>
              ))}
            </div>
          ))}
        </div>

        <section className="layers" aria-label="layers">
          <h3>Layers (top is in front)</h3>
          <ul>
            {order.map(({l, i}) => (
              <li key={l.id} className={selected === l.id ? 'selected' : ''}>
                <button onClick={() => setSelected(l.id)} aria-pressed={selected === l.id}>{layerLabel(l)}</button> <small>{l.id}</small>
                <label>Visible <input type="checkbox" checked={l.visible} disabled={!canEdit} onChange={e => edit(updateLayer(scene, l.id, {visible: e.target.checked}))}/></label>
                {canEdit && <>
                  {(['front', 'forward', 'backward', 'back'] as ZMove[]).map(m => (
                    <button key={m} aria-label={`${m} ${l.id}`} disabled={(m === 'front' || m === 'forward') ? i === scene.layers.length - 1 : i === 0}
                      onClick={() => edit(reorder(scene, l.id, m))}>{{front: 'To front', forward: 'Forward', backward: 'Backward', back: 'To back'}[m]}</button>
                  ))}
                  <button aria-label={'remove ' + l.id} onClick={() => edit(removeLayer(scene, l.id))}>Delete</button>
                </>}
              </li>
            ))}
          </ul>
          {canEdit && <>
            <label>Source <select value={addSource} onChange={e => setAddSource(e.target.value as Source)}>
              {SOURCES.map(s => <option key={s} value={s}>{SOURCE_NAMES[s]}</option>)}</select></label>
            <button onClick={() => { const r = addLayer(scene, addSource); if (!r) { onMessage('At most 16 layers'); return; } edit(r.scene); setSelected(r.id); }}>Add layer</button>
            <button disabled={!v2} onClick={() => { const r = addColorLayer(scene, '#1e3a5f'); if (!r) { onMessage('At most 16 layers'); return; } edit(r.scene); setSelected(r.id); }}>Add colour layer</button>
          </>}
        </section>

        <div className="library">
          <AssetLibrary api={api} canEdit={canEdit} v2={v2} onMessage={onMessage} onAdd={a => {
            const r = addImageLayer(scene, a.id, a.width, a.height);
            if (!r) { onMessage('At most 16 layers'); return; }
            edit(r.scene); setSelected(r.id);
          }}/>
          <TemplatePanel api={api} canEdit={canEdit} v2={v2} onMessage={onMessage} onAdd={(a, def, vars) => {
            const r = addImageLayer(scene, a.id, a.width, a.height, {template: reference(def, vars), base: def.id, x: 80, y: 860});
            if (!r) { onMessage('At most 16 layers'); return; }
            edit(r.scene); setSelected(r.id);
          }}/>
        </div>

        {layer && (
          <section className="props" aria-label="properties">
            <h3>Properties: {layer.id}</h3>
            {(['x', 'y', 'width', 'height'] as const).map(k => (
              <label key={k}>{k}<input type="number" value={layer[k]} disabled={!canEdit} onChange={e => edit(updateLayer(scene, layer.id, {[k]: Number(e.target.value)}))}/></label>
            ))}
            <label>Visible<input type="checkbox" checked={layer.visible} disabled={!canEdit} onChange={e => edit(updateLayer(scene, layer.id, {visible: e.target.checked}))}/></label>
            {v2 && <V2Props api={api} layer={layer} canEdit={canEdit} onMessage={onMessage} onChange={c => edit(updateLayer(scene, layer.id, c as never))}/>}
            <label>Lock aspect ratio<input type="checkbox" checked={lockAspect} onChange={e => setLockAspect(e.target.checked)}/></label>
            <label>Grid <select value={grid} onChange={e => setGrid(Number(e.target.value))}>
              {[1, 8, 16, 40].map(g => <option key={g} value={g}>{g === 1 ? 'off' : g + 'px'}</option>)}</select></label>
          </section>
        )}

        {code !== '' && (
          <section>
            <textarea aria-label="scene json" value={code} onChange={e => setCode(e.target.value)}/>
            <button onClick={() => { const r = validateSceneText(code); if (r.violation) { onMessage('Invalid scene ' + r.violation.path + ': ' + r.violation.reason); return; } edit(r.doc as Scene); setCode(''); }}>Apply to draft</button>
            <button onClick={() => setCode('')}>Close</button>
          </section>
        )}

        {revisions.length > 0 && (
          <section>
            <h2>History (last 50 revisions)</h2>
            <ul>{revisions.map(r => (
              <li key={r.revision}>r{r.revision} {new Date(r.created_at_unix * 1000).toLocaleString()} {canEdit &&
                <button onClick={() => { edit(r.document as Scene); onMessage('r' + r.revision + ' loaded into the draft (not saved yet)'); }}>Restore</button>}</li>
            ))}</ul>
          </section>
        )}
      </div>
    </div>
  );
}
