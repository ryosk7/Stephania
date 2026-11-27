// Stream controls and status (DESIGN.md §9 keeps the full Studio layout for later).
import {Activity, Play, Square} from 'lucide-react';
import {useStudio} from './studio-context';

export function Dashboard() {
  const {api, onMessage} = useStudio();
  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }
  return (
    <section aria-label="dashboard" data-testid="dashboard" className="flex flex-col gap-3 rounded-card border border-border bg-surface p-4">
      <h2 className="m-0">Stream</h2>
      <div className="flex flex-wrap gap-2">
        <button className="flex items-center gap-2" onClick={() => run(async () => { const st = await api.status(); onMessage((st.engine_restarted ? '⚠ The engine restarted (check the state again)\n' : '') + JSON.stringify(st)); })}>
          <Activity size={16} aria-hidden/>Status
        </button>
        <button className="flex items-center gap-2 border-text bg-text text-on-primary hover:bg-text hover:opacity-85" onClick={() => run(async () => onMessage(JSON.stringify(await api.start())))}>
          <Play size={16} aria-hidden/>Start stream
        </button>
        <button className="flex items-center gap-2 border-live bg-live text-white hover:bg-live hover:opacity-85" onClick={() => run(async () => onMessage(JSON.stringify(await api.stop())))}>
          <Square size={14} fill="currentColor" aria-hidden/>Stop
        </button>
      </div>
      <p className="m-0 text-sm text-secondary">Edit scenes in Studio; microphone and HDMI audio in Audio.</p>
    </section>
  );
}
