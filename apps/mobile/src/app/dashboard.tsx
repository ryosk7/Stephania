// Operating the device over the LAN with the enrolled token. Every request
// goes through the native pinned client (modules/t3box-pinned-fetch).
import {router, useLocalSearchParams} from 'expo-router';
import {useCallback, useEffect, useMemo, useState} from 'react';
import {Pressable, ScrollView, Switch, Text, View} from 'react-native';
import PinnedFetch from '../../modules/t3box-pinned-fetch';
import {DeviceClient, DeviceError, type AudioInput, type Status, type StoredScene} from '../net/device';
import {loadEnrollment} from '../state/session';
import {Button, s} from '../ui';

const STATE: Record<string, string> = {idle: '停止中', starting_or_retrying: '開始中・再試行中', native_worker_active: '配信中', stopping: '停止処理中'};

export function describeDeviceError(e: unknown): string {
  if (e instanceof DeviceError) {
    const m: Record<string, string> = {
      unreachable: 'T3Boxに接続できません。同じWi-Fiにいるか確認してください（証明書が一致しない接続には接続しません）。',
      unauthorized: 'このスマホのアクセス権が無効です。Wi-Fi設定画面から再発行してください。',
      program_inactive: '配信していないため切り替えできません。',
      engine_unavailable: 'メディアエンジンが応答していません。',
    };
    return m[e.code] ?? `エラー: ${e.code} ${e.message}`;
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
  const [message, setMessage] = useState('接続しています…');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    loadEnrollment(device).then(e => {
      if (!e) { setMessage('このT3Boxのアクセス権がありません。'); return; }
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
      <Text style={s.p}>Program: {program ? `${program.scene_id} r${program.revision}` : 'HDMI（初期状態）'}</Text>
      {status?.last_error ? <Text style={s.error}>{status.last_error}</Text> : null}
      <Button title={streaming ? '配信を停止' : '配信を開始'} disabled={!client || busy}
        onPress={() => void run(() => (streaming ? client!.stop() : client!.start()), streaming ? '停止を要求しました' : '開始を要求しました')} />

      <Text style={s.h}>シーン</Text>
      <View style={{flexDirection: 'row', alignItems: 'center', gap: 8}}>
        <Switch value={fade} onValueChange={setFade} accessibilityLabel="フェードで切り替え" />
        <Text style={s.p}>フェードで切り替え（0.5秒）</Text>
      </View>
      {sorted.map(sc => (
        <Pressable key={sc.id} accessibilityRole="button" disabled={busy || !streaming} style={s.row}
          onPress={() => void run(() => client!.transition(sc.id, sc.revision, fade ? 500 : undefined), `${sc.document.name} に切り替えました`)}>
          <Text style={[s.p, program?.scene_id === sc.id && {fontWeight: '700'}]}>
            {program?.scene_id === sc.id ? '● ' : ''}{sc.document.name} <Text style={{color: '#888'}}>r{sc.revision}</Text>
          </Text>
        </Pressable>
      ))}
      {!streaming && sorted.length > 0 && <Text style={s.p}>シーンの切り替えは配信中に行えます。</Text>}

      <Text style={s.h}>音声</Text>
      {audio.map(a => (
        <View key={a.id} style={{flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between'}}>
          <Text style={s.p}>{a.id === 'hdmi' ? 'HDMI音声' : 'マイク'}{a.enabled ? '' : '（無効）'}</Text>
          <Switch value={!a.muted} disabled={busy || !a.enabled} accessibilityLabel={(a.id === 'hdmi' ? 'HDMI音声' : 'マイク') + 'を出す'}
            onValueChange={on => void run(() => client!.setMuted(a.id, !on))} />
        </View>
      ))}

      <Button title="PCのブラウザで開く" disabled={!client} onPress={() => router.push({pathname: '/open-on-pc', params: {device}})} />
      {message !== '' && <Text style={s.p}>{message}</Text>}
    </ScrollView>
  );
}
