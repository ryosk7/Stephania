// Setup, Wi-Fi step (skipped when the device is already online). The list
// is what the *device* sees, strongest first: the phone's own Wi-Fi is not
// read, so the app needs no location permission. Tapping a network opens
// the password sheet; joining issues LAN access and opens Control. Before a
// password is sent the person confirms the device: Bluetooth cannot prove
// that the device answering is the one in front of them
// (contracts/ble-protocol.md, "Device identity").
import {router} from 'expo-router';
import {useCallback, useEffect, useState} from 'react';
import {ActivityIndicator, View} from 'react-native';
import type {ScanNetwork} from '@stephania/provisioning-client';
import {Field, Footnote, Icon, IconButton, List, Notice, PrimaryButton, Screen, SectionHeading, Sheet, StatusView, Toggle, TopBar, useColor} from '@/shared/ui';
import {describeSetupError} from './errors';
import {finishSetup} from './flow';
import {activeSession} from './state';

const supported = (n: ScanNetwork) => n.security === 'open' || n.security === 'wpa-psk';
const bars = (rssi: number): 'signal0' | 'signal1' | 'signal2' | 'signal3' => (rssi > -60 ? 'signal3' : rssi > -72 ? 'signal2' : rssi > -82 ? 'signal1' : 'signal0');

export function WifiScreen() {
  const active = activeSession();
  const [networks, setNetworks] = useState<ScanNetwork[] | null>(null);
  const [picked, setPicked] = useState<ScanNetwork>();
  const [psk, setPsk] = useState('');
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [scanError, setScanError] = useState('');
  const [confirmed, setConfirmed] = useState(active?.identityConfirmed === true);
  const spinner = useColor('secondary');

  const scan = useCallback(async () => {
    if (!active) return;
    setNetworks(null); setScanError('');
    try {
      setNetworks([...(await active.session.scan())].sort((a, b) => Number(supported(b)) - Number(supported(a)) || b.rssi - a.rssi));
    } catch (e) { setNetworks([]); setScanError(describeSetupError(e)); }
  }, [active]);
  useEffect(() => { void scan(); }, [scan]);

  if (!active) {
    return <Screen header={<TopBar onBack={() => router.back()} />}>
      <StatusView icon="bluetooth" tone="warning" title="Not connected" body="Open Stephania Connect next to the device to set it up.">
        <PrimaryButton title="Home" icon="bluetooth" onPress={() => router.replace('/')} />
      </StatusView>
    </Screen>;
  }

  async function join() {
    if (!picked) return;
    setError('');
    try {
      setBusy(`Joining ${picked.ssid}`);
      const state = await active!.session.configureWifi(
        picked.security === 'open' ? {ssid: picked.ssid, security: 'open'} : {ssid: picked.ssid, security: 'wpa-psk', psk});
      if (state !== 'CONNECTED') throw Object.assign(new Error('join_failed'), {code: 'join_failed'});
      setPsk('');
      setBusy('Setting up secure access');
      const id = await finishSetup();
      setPicked(undefined);
      router.replace({pathname: '/device/[id]', params: {id}});
    } catch (e) {
      setError(describeSetupError(e));
    } finally {
      setBusy('');
    }
  }

  const secret = picked?.security === 'wpa-psk';
  const canJoin = !!picked && (picked.security === 'open' || (psk.length >= 8 && confirmed));
  const confirm = (v: boolean) => { setConfirmed(v); if (active) active.identityConfirmed = v; };
  return (
    <Screen scroll header={<TopBar onBack={() => router.back()} title={active.name} trailing={<IconButton icon="refresh" label="Scan again" onPress={() => void scan()} />} />}>
      <View className="flex-row items-center gap-2.5 px-1" accessible accessibilityLabel="This phone is the owner">
        <Icon name="check" size={18} strokeWidth={2.6} tone="success" />
        <Footnote>This phone is the owner</Footnote>
      </View>
      <View className="gap-2">
        <SectionHeading icon="wifi" label="Wi-Fi networks, strongest first" />
        <List.Section>
          {networks === null
            ? <List.Row title="Scanning…" accessory={<ActivityIndicator color={spinner} />} />
            : networks.length === 0
              ? <List.Row title="No networks found" />
              : networks.map(n => (
                <List.Row key={n.ssid} title={n.ssid} detail={supported(n) ? undefined : 'Not supported'}
                  accessibilityLabel={`${n.ssid}${n.security !== 'open' ? ', secured' : ''}${supported(n) ? '' : ', security not supported'}`}
                  accessory={<View className="flex-row items-center gap-2">
                    {n.security !== 'open' && <Icon name="lock" size={14} color={supported(n) ? 'text' : 'tertiary'} />}
                    <Icon name={bars(n.rssi)} size={16} color={supported(n) ? 'text' : 'tertiary'} />
                  </View>}
                  disabled={!supported(n)} onPress={() => { setError(''); setPsk(''); setPicked(n); }} />
              ))}
        </List.Section>
      </View>
      {scanError !== '' && <Notice tone="warning">{scanError}</Notice>}
      <View className="flex-row items-center justify-center gap-2" accessible accessibilityLabel="With a LAN cable this step is skipped">
        <Icon name="cable" size={16} color="tertiary" /><Footnote tone="tertiary">With a LAN cable this step is skipped</Footnote>
      </View>

      <Sheet visible={!!picked} icon="wifi" title={picked?.ssid} subtitle={`${active.name} joins this network`}
        onClose={() => { if (!busy) setPicked(undefined); }}>
        {picked?.security === 'wpa-psk' && (
          <Field label={`Password for ${picked.ssid}`} icon="lock" secure autoFocus value={psk} onChangeText={setPsk}
            placeholder="At least 8 characters" returnKeyType="join" onSubmitEditing={() => canJoin && void join()} />
        )}
        {secret && (
          <>
            <Notice tone="neutral" icon="bluetooth">{`The password is sent to ${active.name} over Bluetooth, which cannot prove which device that is. A LAN cable avoids sending it at all.`}</Notice>
            <View className="flex-row items-center gap-3 px-1">
              <Footnote className="flex-1">{`${active.name} is my device, right here, and the only Stephania nearby`}</Footnote>
              <Toggle value={confirmed} onChange={confirm} label={`${active.name} is my device`} />
            </View>
          </>
        )}
        {error !== '' && <Notice tone="warning">{error}</Notice>}
        {busy !== '' && <Footnote className="text-center">{busy}…</Footnote>}
        <PrimaryButton title="Join" busy={busy !== ''} disabled={!canJoin} onPress={() => void join()} />
      </Sheet>
    </Screen>
  );
}
