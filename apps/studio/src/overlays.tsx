// Scene v2 editing UI: asset library, overlay templates (rendered to PNG in
// the browser) and the v2 layer properties (opacity, rotation, fit, crop,
// colour, template variables).
import React, {useEffect, useRef, useState} from 'react';
import type {Asset, createClient, OverlayTemplate} from './api/client';
import {isVideo, type Crop, type Fit, type ImageLayer, type Layer, type Scene, type VideoLayer} from './editor';
import {BUILTIN, checkDeveloperSvg, initialVars, reference, renderPng, svgSize, templateFor, templateSvg, type TemplateDef} from './templates';

type Api = ReturnType<typeof createClient>;

/** Object URLs for asset ids, fetched with the page's credentials. */
export function useAssetUrls(api: Api, ids: string[]): Record<string, string> {
  const cache = useRef<Record<string, string>>({});
  const [, bump] = useState(0);
  const key = [...new Set(ids)].sort().join(',');
  useEffect(() => {
    let alive = true;
    for (const id of key ? key.split(',') : []) {
      if (cache.current[id]) continue;
      cache.current[id] = 'pending';
      api.assetBlob(id).then(b => { cache.current[id] = URL.createObjectURL(b); if (alive) bump(n => n + 1); })
        .catch(() => { delete cache.current[id]; });
    }
    return () => { alive = false; };
  }, [api, key]);
  useEffect(() => () => { for (const u of Object.values(cache.current)) if (u.startsWith('blob:')) URL.revokeObjectURL(u); }, []);
  return Object.fromEntries(Object.entries(cache.current).filter(([, u]) => u !== 'pending'));
}

export function toDef(t: OverlayTemplate): TemplateDef {
  return {id: t.id, name: t.name, width: t.width, height: t.height, vars: t.vars, svg: t.svg};
}

/** Render a template and upload the PNG; returns the stored asset. */
export async function renderTemplate(api: Api, def: TemplateDef, vars: Record<string, string>): Promise<Asset> {
  const svg = templateSvg(def, vars);
  const problem = def.svg ? checkDeveloperSvg(svg) : null;
  if (problem) throw new Error(problem);
  const png = await renderPng(svg, def.width, def.height);
  return api.uploadAsset(png, def.id + '.png');
}

export function AssetLibrary({api, canEdit, v2, onAdd, onMessage}: {
  api: Api; canEdit: boolean; v2: boolean; onAdd: (a: Asset) => void; onMessage: (m: string) => void;
}) {
  const [assets, setAssets] = useState<Asset[]>([]);
  const urls = useAssetUrls(api, assets.map(a => a.id));
  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }
  const load = async () => setAssets(await api.assets());
  useEffect(() => { void run(load); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <section aria-label="assets" className="assets">
      <h3>画像アセット</h3>
      {canEdit && <label className="file">PNGをアップロード <input type="file" accept="image/png" onChange={e => {
        const f = e.target.files?.[0];
        e.target.value = '';
        if (f) void run(async () => { const a = await api.uploadAsset(f, f.name); await load(); onMessage(`アップロードしました（${a.id}）`); });
      }}/></label>}
      <ul>
        {assets.map(a => (
          <li key={a.id} data-testid={'asset-' + a.id}>
            {urls[a.id] && <img src={urls[a.id]} alt={a.name} width={48} height={Math.max(1, Math.round(48 * a.height / a.width))}/>}
            <span>{a.name} <small>{a.width}×{a.height}</small></span>
            {canEdit && <button disabled={!v2} title={v2 ? '' : 'v2 シーンのみ'} onClick={() => onAdd(a)}>シーンに追加</button>}
            {canEdit && <button aria-label={'delete asset ' + a.name} onClick={() => void run(async () => { await api.deleteAsset(a.id); await load(); onMessage('削除しました'); })}>削除</button>}
          </li>
        ))}
      </ul>
    </section>
  );
}

export function TemplatePanel({api, canEdit, v2, onAdd, onMessage}: {
  api: Api; canEdit: boolean; v2: boolean; onAdd: (a: Asset, def: TemplateDef, vars: Record<string, string>) => void; onMessage: (m: string) => void;
}) {
  const [custom, setCustom] = useState<TemplateDef[]>([]);
  const all = [...BUILTIN, ...custom];
  const [chosen, setChosen] = useState(BUILTIN[0]!.id);
  const def = all.find(t => t.id === chosen) ?? BUILTIN[0]!;
  const [vars, setVars] = useState<Record<string, string>>(() => initialVars(BUILTIN[0]!));
  const [dev, setDev] = useState({id: '', name: '', svg: '', vars: '[]'});
  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }
  const load = async () => setCustom((await api.templates()).map(toDef));
  useEffect(() => { void run(load); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  if (!canEdit) return null;

  return (
    <section aria-label="templates" className="templates">
      <h3>テキスト・テンプレート</h3>
      <label>テンプレート <select value={def.id} onChange={e => { const d = all.find(t => t.id === e.target.value)!; setChosen(d.id); setVars(initialVars(d)); }}>
        {all.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}</select></label>
      {def.vars.map(v => (
        <label key={v.key}>{v.label} <input value={vars[v.key] ?? ''} maxLength={200} onChange={e => setVars({...vars, [v.key]: e.target.value})}/></label>
      ))}
      <button disabled={!v2} title={v2 ? '' : 'v2 シーンのみ'} onClick={() => void run(async () => {
        const a = await renderTemplate(api, def, vars);
        onAdd(a, def, vars);
        onMessage(`「${def.name}」を追加しました`);
      })}>テンプレートを追加</button>
      <details>
        <summary>開発者モード: SVG テンプレート</summary>
        <p>{'{{変数名}}'} が置き換わります。スクリプト・イベント属性・外部参照は使えません（ブラウザ内で画像として描画されます）。</p>
        <label>ID <input value={dev.id} onChange={e => setDev({...dev, id: e.target.value})} placeholder="my-badge"/></label>
        <label>名前 <input value={dev.name} onChange={e => setDev({...dev, name: e.target.value})}/></label>
        <textarea aria-label="template svg" value={dev.svg} onChange={e => setDev({...dev, svg: e.target.value})}
          placeholder={'<svg xmlns="http://www.w3.org/2000/svg" width="600" height="120">…{{speaker.name}}…</svg>'}/>
        <label>変数 (JSON) <input value={dev.vars} onChange={e => setDev({...dev, vars: e.target.value})} placeholder='[{"key":"speaker.name","label":"名前","initial":""}]'/></label>
        <button onClick={() => void run(async () => {
          const problem = checkDeveloperSvg(dev.svg);
          if (problem) { onMessage(problem); return; }
          const size = svgSize(dev.svg);
          if (!size) { onMessage('<svg> に width と height（1920×1080 以内）を指定してください'); return; }
          let v: TemplateDef['vars'];
          try { v = JSON.parse(dev.vars); } catch { onMessage('変数の JSON が正しくありません'); return; }
          await api.saveTemplate({id: dev.id, name: dev.name, width: size.width, height: size.height, svg: dev.svg, vars: v});
          await load();
          setChosen(dev.id);
          setVars(Object.fromEntries(v.map(x => [x.key, x.initial])));
          onMessage('テンプレートを保存しました');
        })}>テンプレートを保存</button>
      </details>
    </section>
  );
}

const FITS: Fit[] = ['stretch', 'contain', 'cover'];
const FIT_NAMES: Record<Fit, string> = {stretch: '引き伸ばし', contain: '全体を表示', cover: '画面を埋める'};

export function V2Props({api, layer, canEdit, onChange, onMessage}: {
  api: Api; layer: Layer; canEdit: boolean; onChange: (c: Partial<Layer>) => void; onMessage: (m: string) => void;
}) {
  const [vars, setVars] = useState<Record<string, string>>({});
  const tpl = layer.type === 'image' ? layer.template : undefined;
  useEffect(() => { setVars(tpl?.vars ?? {}); }, [layer.id, tpl]);
  const [custom, setCustom] = useState<TemplateDef[]>([]);
  useEffect(() => { if (tpl && !templateFor(tpl.id)) api.templates().then(t => setCustom(t.map(toDef))).catch(() => undefined); }, [api, tpl]);
  const crop: Crop = (isVideo(layer) && layer.crop) || {left: 0, top: 0, right: 0, bottom: 0};
  const pct = (v: number) => Math.round(v / 100);
  return (
    <div className="v2props">
      <label>不透明度 <input type="range" aria-label="opacity slider" min={0} max={100} value={layer.opacity ?? 100} disabled={!canEdit}
        onChange={e => onChange({opacity: Number(e.target.value)})}/>
        <input type="number" aria-label="opacity" min={0} max={100} value={layer.opacity ?? 100} disabled={!canEdit}
          onChange={e => onChange({opacity: Number(e.target.value)})}/>%</label>
      <label>回転 <input type="number" aria-label="rotation" min={-180} max={180} value={layer.rotation ?? 0} disabled={!canEdit}
        onChange={e => onChange({rotation: Number(e.target.value)})}/>°</label>
      {layer.type !== 'color' && (
        <label>フィット <select aria-label="fit" value={(layer as VideoLayer | ImageLayer).fit ?? 'stretch'} disabled={!canEdit} onChange={e => onChange({fit: e.target.value as Fit})}>
          {FITS.map(f => <option key={f} value={f}>{FIT_NAMES[f]}</option>)}</select></label>
      )}
      {isVideo(layer) && (['left', 'top', 'right', 'bottom'] as const).map(k => (
        <label key={k}>{{left: '左', top: '上', right: '右', bottom: '下'}[k]}トリミング <input type="number" aria-label={'crop ' + k} min={0} max={90} value={pct(crop[k])} disabled={!canEdit}
          onChange={e => onChange({crop: {...crop, [k]: Math.round(Number(e.target.value) * 100)}} as Partial<VideoLayer>)}/>%</label>
      ))}
      {layer.type === 'color' && (
        <label>色 <input type="color" aria-label="color" value={layer.color.toLowerCase()} disabled={!canEdit} onChange={e => onChange({color: e.target.value})}/></label>
      )}
      {tpl && (() => {
        const def = templateFor(tpl.id, custom);
        if (!def) return <p>テンプレート「{tpl.id}」が見つかりません</p>;
        return (
          <fieldset>
            <legend>{def.name}</legend>
            {def.vars.map(v => (
              <label key={v.key}>{v.label} <input aria-label={'var ' + v.key} value={vars[v.key] ?? ''} maxLength={200} disabled={!canEdit} onChange={e => setVars({...vars, [v.key]: e.target.value})}/></label>
            ))}
            {canEdit && <button onClick={() => void (async () => {
              try {
                const a = await renderTemplate(api, def, vars);
                onChange({asset: a.id, template: reference(def, vars)} as Partial<ImageLayer>);
                onMessage('テンプレートを再描画しました（保存・適用で反映）');
              } catch (e) { onMessage(String(e)); }
            })()}>再描画</button>}
          </fieldset>
        );
      })()}
    </div>
  );
}

/** CSS for previewing a layer on the canvas (draft view, not Program). */
export function layerStyle(l: Layer): React.CSSProperties {
  return {
    left: l.x / 19.2 + '%', top: l.y / 10.8 + '%', width: l.width / 19.2 + '%', height: l.height / 10.8 + '%',
    opacity: (l.opacity ?? 100) / 100,
    transform: l.rotation ? `rotate(${l.rotation}deg)` : undefined,
    background: l.type === 'color' ? l.color : l.type === 'image' ? 'transparent' : undefined,
  };
}

export {reference} from './templates';
