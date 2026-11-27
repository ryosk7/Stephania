// Setup step 3: join Wi-Fi, then issue LAN management access (sealed under
// the owner key; never over plain BLE). Ends on a success state.
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useCallback, useEffect, useState} from 'react';
import {ActivityIndicator, View} from 'react-native';
import type {ScanNetwork} from '@t3box/provisioning-client';
import {saveEnrollment} from '@/shared/device';
import {Field, Footnote, List, Notice, PrimaryButton, Screen, StatusView, TextButton, Title, color, Icon, TopBar} from '@/shared/ui';
import {describeSetupError} from './errors';
import {activeSession} from './state';

type Step = {kind: 'pick'} | {kind: 'joining'} | {kind: 'issuing'} | {kind: 'done'; url: string};

export function WifiScreen() {
  const active = activeSession();
  const [networks, setNetworks] = useState<ScanNetwork[] | null>(null);
  const [selected, setSelected] = useState<ScanNetwork>();
  const [psk, setPsk] = useState('');
  const [step, setStep] = useState<Step>({kind: 'pick'});
  const [error, setError] = useState('');

  const scan = useCallback(async () => {
    if (!active) return;
    setNetworks(null); setError('');
    try { setNetworks(await active.session.scan()); } catch (e) { setNetworks([]); setError(describeSetupError(e)); }
  }, [active]);
  useEffect(() => { void scan(); }, [scan]);

  if (!active) {
    return <Screen header={<TopBar onBack={() => router.back()} title="Wi-Fi" />}><StatusView icon={'bluetooth'} tone="warning" title="Not connected" body="Open the T3Box from the nearby list first.">
      <PrimaryButton title="Find nearby T3Box" onPress={() => router.replace('/setup/nearby')} />
    </StatusView></Screen>;
  }

  /** Joins `selected` (unless `skipJoin`: the T3Box is already on the LAN), then issues access. */
  async function join(skipJoin = false) {
    if (!skipJoin && !selected) return;
    setError('');
    try {
      if (!skipJoin && selected) {
        setStep({kind: 'joining'});
        const state = await active!.session.configureWifi(
          selected.security === 'open' ? {ssid: selected.ssid, security: 'open'} : {ssid: selected.ssid, security: 'wpa-psk', psk});
        setPsk('');
        if (state !== 'CONNECTED') throw Object.assign(new Error('join_failed'), {code: 'join_failed'});
      }
      setStep({kind: 'issuing'});
      if (!(await active!.session.managementAvailable())) {
        throw new Error('The T3Box is still preparing its certificate. Try again in a few minutes.');
      }
      const e = await active!.session.enrollManagement();
      await saveEnrollment(active!.deviceId, e, active!.name);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      setStep({kind: 'done', url: e.urls[0]!});
    } catch (e) {
      setStep({kind: 'pick'});
      setError(describeSetupError(e));
    }
  }

  if (step.kind === 'joining' || step.kind === 'issuing') {
    return <Screen header={<TopBar onBack={() => router.back()} title="Wi-Fi" />}><StatusView icon={'wifi'} tone="info"
      title={step.kind === 'joining' ? `Joining ${selected?.ssid}…` : 'Setting up secure access…'}
      body={step.kind === 'joining' ? 'This can take up to 30 seconds.' : 'The T3Box hands this phone its address and certificate.'}>
      <ActivityIndicator color={color.textSecondary} />
    </StatusView></Screen>;
  }
  if (step.kind === 'done') {
    return <Screen header={<TopBar onBack={() => router.back()} title="Wi-Fi" />} footer={<PrimaryButton title="Open controls" onPress={() => router.replace({pathname: '/device/[id]', params: {id: active.deviceId}})} />}>
      <StatusView icon={'done'} tone="success" title={`${active.name} is ready`} body={`Reachable at ${step.url}`} />
    </Screen>;
  }

  const supported = (n: ScanNetwork) => n.security === 'open' || n.security === 'wpa-psk';
  const canJoin = !!selected && supported(selected) && (selected.security === 'open' || psk.length >= 8);
  return (
    <Screen header={<TopBar onBack={() => router.back()} title="Wi-Fi" />} scroll footer={<PrimaryButton title="Join" disabled={!canJoin} onPress={() => void join()} />}>
      <Title>Choose Wi-Fi for {active.name}</Title>
      <Footnote>Use the network the phone and the PC are on.</Footnote>
      <List.Section title="Networks">
        {networks === null
          ? <List.Row title="Scanning…" accessory={<ActivityIndicator color={color.textSecondary} />} />
          : networks.length === 0
            ? <List.Row title="No networks found" />
            : networks.map(n => (
              // iOS Wi-Fi list: check on the left of the chosen network, lock and signal on the right.
              <List.Row key={n.ssid} title={n.ssid} detail={supported(n) ? undefined : 'Security not supported'}
                leading={<View style={{width: 20}}>{selected?.ssid === n.ssid && <Icon name="check" size={18} strokeWidth={2.5} />}</View>}
                accessory={<View style={{flexDirection: 'row', gap: 8, alignItems: 'center'}}>
                  {n.security !== 'open' && <Icon name={'lock'} size={14} tint={supported(n) ? color.text : color.textTertiary} />}
                  <Icon name={'wifi'} size={16} tint={supported(n) ? color.text : color.textTertiary} />
                </View>}
                selected={selected?.ssid === n.ssid} disabled={!supported(n)} onPress={() => setSelected(n)} />
            ))}
      </List.Section>
      {selected?.security === 'wpa-psk' && (
        <Field label={`Password for ${selected.ssid}`} secure value={psk} onChangeText={setPsk} placeholder="At least 8 characters" returnKeyType="join" onSubmitEditing={() => canJoin && void join()} />
      )}
      {error !== '' && <Notice tone="warning">{error}</Notice>}
      <TextButton title="Scan again" onPress={() => void scan()} />
      <TextButton title="Already on this network? Set up access only" onPress={() => void join(true)} />
    </Screen>
  );
}
