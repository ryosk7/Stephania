// Program audio panel (docs/AUDIO.md): per-input state, live meters from the
// audio.levels event, gain, mute, enable and ALSA device.
import React, {useEffect, useRef, useState} from 'react';
import type {AudioInput, createClient} from './api/client';
import type {AudioLevels, Level} from './live';

type Api = ReturnType<typeof createClient>;
const NAMES: Record<string, string> = {hdmi: 'HDMI音声', mic: 'マイク'};
const STATES: Record<string, string> = {
  disabled: '無効', idle: '待機（配信停止中）', waiting: '接続待ち', active: '入力あり', stalled: '入力が止まっています', error: 'エラー（自動再接続中）',
};

/** dBFS (−60..0) → 0..100 % for meter bars. */
export function meterPercent(dbfs: number | undefined): number {
  if (dbfs === undefined || !Number.isFinite(dbfs)) return 0;
  return Math.max(0, Math.min(100, ((dbfs + 60) / 60) * 100));
}

function Meter({level, label}: {level?: Level; label: string}) {
  return (
    <div className="meter" role="meter" aria-label={label} aria-valuemin={-60} aria-valuemax={0}
      aria-valuenow={level ? Math.round(level.peak_dbfs) : -60}>
      <div className="rms" style={{width: meterPercent(level?.rms_dbfs) + '%'}}/>
      <div className="peak" style={{left: meterPercent(level?.peak_dbfs) + '%'}}/>
      {level?.clipped && <span className="clip">CLIP</span>}
    </div>
  );
}

export function AudioPanel({api, levels, onMessage}: {api: Api; levels?: AudioLevels; onMessage: (m: string) => void}) {
  const [inputs, setInputs] = useState<AudioInput[]>([]);
  const [state, setState] = useState<Record<string, {state: string; error?: string}>>({});
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  async function load() {
    const r = await api.audio();
    const live = (r.engine?.inputs ?? []) as (AudioInput & {state: string; error?: string})[];
    const byId = new Map<string, AudioInput>();
    for (const i of live) byId.set(i.id, {id: i.id, enabled: i.enabled, device: i.device, channels: i.channels, gain_db: i.gain_db, muted: i.muted});
    for (const i of r.stored) byId.set(i.id, i);
    setInputs(['hdmi', 'mic'].map(id => byId.get(id)).filter((i): i is AudioInput => !!i));
    setState(Object.fromEntries(live.map(i => [i.id, {state: i.state, error: i.error}])));
  }
  useEffect(() => { load().catch(e => onMessage(String(e))); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function save(id: string, change: Partial<AudioInput>) {
    try {
      const r = await api.saveAudio({inputs: [{id: id as 'hdmi' | 'mic', ...change}]});
      setInputs(r.stored);
      onMessage(r.applied ? '音声設定を適用しました' : '保存しました（エンジン接続時に適用されます）');
    } catch (e) {
      onMessage(String(e));
      await load();
    }
  }
  function edit(id: string, change: Partial<AudioInput>, live = false) {
    setInputs(xs => xs.map(i => (i.id === id ? {...i, ...change} : i)));
    if (live) {
      // Gain slider: apply live, at most every 150 ms.
      clearTimeout(timer.current);
      timer.current = setTimeout(() => void save(id, change), 150);
    }
  }

  return (
    <section aria-label="audio" data-testid="audio">
      <h2>音声</h2>
      {levels?.master && <Meter level={levels.master} label="program"/>}
      {inputs.map(i => {
        const live = levels?.inputs.find(l => l.id === i.id);
        const st = live?.state ?? state[i.id]?.state;
        return (
          <div key={i.id} className="audio-input" data-testid={'audio-' + i.id}>
            <strong>{NAMES[i.id] ?? i.id}</strong> <span data-testid={'audio-state-' + i.id}>{st ? STATES[st] ?? st : ''}</span>
            {state[i.id]?.error && <span> {state[i.id]!.error}</span>}
            <Meter level={live?.levels} label={i.id}/>
            <label>有効 <input type="checkbox" checked={i.enabled} onChange={e => void save(i.id, {enabled: e.target.checked, device: i.device})}/></label>
            <label>デバイス <input value={i.device} onChange={e => edit(i.id, {device: e.target.value})} onBlur={e => void save(i.id, {device: e.target.value})} placeholder="plughw:CARD=Mic,DEV=0"/></label>
            <label>ゲイン {i.gain_db.toFixed(1)} dB <input type="range" min={-60} max={12} step={0.5} value={i.gain_db}
              aria-label={'gain ' + i.id} onChange={e => edit(i.id, {gain_db: Number(e.target.value)}, true)}/></label>
            <button aria-pressed={i.muted} onClick={() => void save(i.id, {muted: !i.muted})}>{i.muted ? 'ミュート解除' : 'ミュート'}</button>
          </div>
        );
      })}
    </section>
  );
}
