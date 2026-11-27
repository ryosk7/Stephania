import {useEffect, useState} from 'react';
import {FlatList, Pressable, Text, TextInput} from 'react-native';
import type {ScanNetwork} from '@t3box/provisioning-client';
import {activeSession, saveEnrollment} from '../state/session';
import {Button, describeError, Screen, s} from '../ui';

export default function Wifi() {
  const active = activeSession();
  const [networks, setNetworks] = useState<ScanNetwork[]>([]);
  const [selected, setSelected] = useState<ScanNetwork>();
  const [psk, setPsk] = useState('');
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);

  async function run(label: string, f: () => Promise<void>) {
    setBusy(true); setStatus(label);
    try { await f(); } catch (e) { setStatus(describeError(e)); } finally { setBusy(false); }
  }

  const scan = () => run('Wi-Fiを検索しています…', async () => {
    setNetworks(await active!.session.scan());
    setStatus('');
  });

  useEffect(() => { if (active) void scan(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (!active) return <Screen><Text style={s.p}>T3Boxに接続していません。</Text></Screen>;

  const join = () => run('接続しています（最大30秒）…', async () => {
    const state = await active.session.configureWifi(
      selected!.security === 'open' ? {ssid: selected!.ssid, security: 'open'} : {ssid: selected!.ssid, security: 'wpa-psk', psk},
    );
    setPsk('');
    if (state === 'CONNECTED') setStatus('接続しました。続けて管理画面へのアクセスを発行できます。');
    else throw Object.assign(new Error('join_failed'), {code: 'join_failed'});
  });

  // Owner-only: the device issues a LAN token and delivers the URLs and its
  // CA fingerprint sealed under the owner key (never over plain BLE).
  const enroll = () => run('管理画面へのアクセスを発行しています…', async () => {
    if (!(await active.session.managementAvailable())) {
      setStatus('T3Boxが安全な接続（TLS証明書）を準備中です。数分後にもう一度お試しください。');
      return;
    }
    const e = await active.session.enrollManagement();
    await saveEnrollment(active.deviceId, e);
    setStatus(`発行しました: ${e.urls[0]}\n証明書の指紋 ${e.caSha256.slice(0, 16)}…（アプリ内に安全に保存）`);
  });

  return (
    <Screen>
      <Text style={s.h}>{active.name}</Text>
      <FlatList
        data={networks}
        keyExtractor={n => n.ssid}
        renderItem={({item}) => (
          <Pressable style={s.row} onPress={() => setSelected(item)} accessibilityRole="button">
            <Text style={[s.p, selected?.ssid === item.ssid && {fontWeight: '700'}]}>
              {item.ssid}　{item.security === 'open' ? '（保護なし）' : item.security === 'wpa-psk' ? '🔒' : '（非対応）'}
            </Text>
          </Pressable>
        )}
      />
      {selected?.security === 'wpa-psk' && (
        <TextInput style={s.input} value={psk} onChangeText={setPsk} placeholder="パスワード" secureTextEntry autoCapitalize="none" autoCorrect={false} />
      )}
      <Button title="このWi-Fiに接続" disabled={busy || !selected || !['open', 'wpa-psk'].includes(selected.security)} onPress={() => void join()} />
      <Button title="再検索" disabled={busy} onPress={() => void scan()} />
      <Button title="管理画面へのアクセスを発行" disabled={busy} onPress={() => void enroll()} />
      {status !== '' && <Text style={s.p}>{status}</Text>}
    </Screen>
  );
}
