// Connection, engine state and program at a glance; state colour plus words.
import {Radio, TriangleAlert, Wifi, WifiOff} from 'lucide-react';
import type {ConnectionState} from './api/events';
import type {LiveState} from './live';

const connText = {off: 'not connected', connecting: 'connecting…', open: 'live', closed: 'reconnecting'} as const;

export function LiveBar({live, conn}: {live: LiveState; conn: ConnectionState | 'off'}) {
  const ConnIcon = conn === 'open' ? Wifi : WifiOff;
  const onAir = live.engine === 'connected' && live.streamState === 'native_worker_active';
  return (
    <section aria-label="live" data-testid="live" className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-card border border-border bg-surface px-4 py-2.5 text-sm">
      <span className="flex items-center gap-1.5">
        <ConnIcon size={16} aria-hidden className={conn === 'open' ? 'text-success' : 'text-warning'}/>
        <span data-testid="conn">{connText[conn]}</span>
      </span>
      <span className="flex items-center gap-1.5">
        <Radio size={16} aria-hidden className={onAir ? 'text-live' : 'text-secondary'}/>
        Engine: <span data-testid="engine">{live.engine === 'connected' ? live.streamState : live.engine === 'unavailable' ? 'not responding' : 'unknown'}</span>
      </span>
      <span>Program: <span data-testid="program" className="font-mono">{live.program ? live.program.scene_id + ' r' + live.program.revision : 'none'}</span></span>
      {live.lastError && <span data-testid="last-error" className="text-warning"> Error: {live.lastError}</span>}
      {live.engineRestarted && <span data-testid="restarted" className="flex items-center gap-1.5 text-warning"><TriangleAlert size={16} aria-hidden/> The engine restarted</span>}
    </section>
  );
}
