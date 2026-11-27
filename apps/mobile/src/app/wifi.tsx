import {router} from 'expo-router';
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

  const scan = () => run('Scanning for Wi-Fi networks…', async () => {
    setNetworks(await active!.session.scan());
    setStatus('');
  });

  useEffect(() => { if (active) void scan(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (!active) return <Screen><Text style={s.p}>Not connected to a T3Box.</Text></Screen>;

  const join = () => run('Joining (up to 30 s)…', async () => {
    const state = await active.session.configureWifi(
      selected!.security === 'open' ? {ssid: selected!.ssid, security: 'open'} : {ssid: selected!.ssid, security: 'wpa-psk', psk},
    );
    setPsk('');
    if (state === 'CONNECTED') setStatus('Connected. You can now issue management access.');
    else throw Object.assign(new Error('join_failed'), {code: 'join_failed'});
  });

  // Owner-only: the device issues a LAN token and delivers the URLs and its
  // CA fingerprint sealed under the owner key (never over plain BLE).
  const enroll = () => run('Issuing management access…', async () => {
    if (!(await active.session.managementAvailable())) {
      setStatus('The T3Box is still preparing its TLS certificate. Try again in a few minutes.');
      return;
    }
    const e = await active.session.enrollManagement();
    await saveEnrollment(active.deviceId, e, active.name);
    setStatus(`Issued: ${e.urls[0]}\nCertificate fingerprint ${e.caSha256.slice(0, 16)}… (stored securely in the app)`);
    router.push({pathname: '/dashboard', params: {device: active.deviceId}});
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
              {item.ssid} {item.security === 'open' ? '(open)' : item.security === 'wpa-psk' ? '🔒' : '(unsupported)'}
            </Text>
          </Pressable>
        )}
      />
      {selected?.security === 'wpa-psk' && (
        <TextInput style={s.input} value={psk} onChangeText={setPsk} placeholder="Password" secureTextEntry autoCapitalize="none" autoCorrect={false} />
      )}
      <Button title="Join this network" disabled={busy || !selected || !['open', 'wpa-psk'].includes(selected.security)} onPress={() => void join()} />
      <Button title="Scan again" disabled={busy} onPress={() => void scan()} />
      <Button title="Issue management access" disabled={busy} onPress={() => void enroll()} />
      {status !== '' && <Text style={s.p}>{status}</Text>}
    </Screen>
  );
}
