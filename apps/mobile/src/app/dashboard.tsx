// Operating the device over the LAN with the enrolled token. Every request
// goes through the native pinned client (modules/t3box-pinned-fetch).
import {router, useLocalSearchParams} from 'expo-router';
import {useCallback, useEffect, useMemo, useState} from 'react';
import {Pressable, ScrollView, Switch, Text, View} from 'react-native';
import PinnedFetch from '../../modules/t3box-pinned-fetch';
import {DeviceClient, DeviceError, type AudioInput, type Status, type StoredScene} from '../net/device';
import {loadEnrollment} from '../state/session';
import {Button, s} from '../ui';

const STATE: Record<string, string> = {idle: 'Stopped', starting_or_retrying: 'Starting / retrying', native_worker_active: 'Streaming', stopping: 'Stopping'};

export function describeDeviceError(e: unknown): string {
  if (e instanceof DeviceError) {
    const m: Record<string, string> = {
      unreachable: 'Cannot reach the T3Box. Check that you are on the same Wi-Fi (connections with a different certificate are refused).',
      unauthorized: 'This phone\'s access is no longer valid. Issue it again from Wi-Fi setup.',
      program_inactive: 'Not streaming, so the scene cannot be switched.',
      engine_unavailable: 'The media engine is not responding.',
    };
    return m[e.code] ?? `Error: ${e.code} ${e.message}`;
  }
  return String(e);
}

export default function Dashboard() {
  const {device} = useLocalSearchParams<{device: string}>();
  const [client, setClient] = useState<DeviceClient>();
  const [status, setStatus] = useState<Status>();
  const [scenes, setScenes] = useState<StoredScene[]>([]);
  const [audio, setAudio] = useState<AudioInput[]>([]);
  const [fade, setFade] = useState(true);
  const [message, setMessage] = useState('Connecting…');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    loadEnrollment(device).then(e => {
      if (!e) { setMessage('This phone has no access to this T3Box.'); return; }
      setClient(new DeviceClient(e, PinnedFetch));
    }).catch(e => setMessage(String(e)));
  }, [device]);

  const refresh = useCallback(async (c: DeviceClient) => {
    const [st, sc, au] = await Promise.all([c.status(), c.scenes(), c.audio()]);
    setStatus(st); setScenes(sc); setAudio(au.stored); setMessage('');
  }, []);

  useEffect(() => {
    if (!client) return;
    let alive = true;
    const tick = () => refresh(client).catch(e => alive && setMessage(describeDeviceError(e)));
    void tick();
    const t = setInterval(() => void client.status().then(st => alive && setStatus(st)).catch(e => alive && setMessage(describeDeviceError(e))), 2000);
    return () => { alive = false; clearInterval(t); };
  }, [client, refresh]);

  async function run(f: () => Promise<unknown>, done?: string) {
    if (!client) return;
    setBusy(true);
    try { await f(); setMessage(done ?? ''); await refresh(client); } catch (e) { setMessage(describeDeviceError(e)); } finally { setBusy(false); }
  }

  const streaming = status?.streaming_requested === true;
  const program = status?.program;
  const sorted = useMemo(() => [...scenes].sort((a, b) => a.document.name.localeCompare(b.document.name)), [scenes]);

  return (
    <ScrollView contentContainerStyle={s.screen}>
      <Text style={s.h}>{status ? STATE[status.state] ?? status.state : '—'}</Text>
      <Text style={s.p}>Program: {program ? `${program.scene_id} r${program.revision}` : 'HDMI (default)'}</Text>
      {status?.last_error ? <Text style={s.error}>{status.last_error}</Text> : null}
      <Button title={streaming ? 'Stop stream' : 'Start stream'} disabled={!client || busy}
        onPress={() => void run(() => (streaming ? client!.stop() : client!.start()), streaming ? 'Stop requested' : 'Start requested')} />

      <Text style={s.h}>Scenes</Text>
      <View style={{flexDirection: 'row', alignItems: 'center', gap: 8}}>
        <Switch value={fade} onValueChange={setFade} accessibilityLabel="Switch with a fade" />
        <Text style={s.p}>Switch with a 0.5 s fade</Text>
      </View>
      {sorted.map(sc => (
        <Pressable key={sc.id} accessibilityRole="button" disabled={busy || !streaming} style={s.row}
          onPress={() => void run(() => client!.transition(sc.id, sc.revision, fade ? 500 : undefined), `Switched to ${sc.document.name}`)}>
          <Text style={[s.p, program?.scene_id === sc.id && {fontWeight: '700'}]}>
            {program?.scene_id === sc.id ? '● ' : ''}{sc.document.name} <Text style={{color: '#888'}}>r{sc.revision}</Text>
          </Text>
        </Pressable>
      ))}
      {!streaming && sorted.length > 0 && <Text style={s.p}>Scenes can be switched while streaming.</Text>}

      <Text style={s.h}>Audio</Text>
      {audio.map(a => (
        <View key={a.id} style={{flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between'}}>
          <Text style={s.p}>{a.id === 'hdmi' ? 'HDMI audio' : 'Microphone'}{a.enabled ? '' : ' (disabled)'}</Text>
          <Switch value={!a.muted} disabled={busy || !a.enabled} accessibilityLabel={(a.id === 'hdmi' ? 'HDMI audio' : 'Microphone') + ' on air'}
            onValueChange={on => void run(() => client!.setMuted(a.id, !on))} />
        </View>
      ))}

      <Button title="Open in a PC browser" disabled={!client} onPress={() => router.push({pathname: '/open-on-pc', params: {device}})} />
      {message !== '' && <Text style={s.p}>{message}</Text>}
    </ScrollView>
  );
}
