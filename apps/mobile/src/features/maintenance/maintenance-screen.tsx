// Maintenance code for one device (docs/MAINTENANCE.md). The laptop CLI
// (`stephctl ssh`) asks for this code to open a short maintenance window;
// the code is shown only after Face ID or the passcode, and the CA
// fingerprint is shown in the form `stephctl login` prints, to compare.
import * as Clipboard from 'expo-clipboard';
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {View} from 'react-native';
import {shortFingerprint} from '@stephania/provisioning-client';
import {DeviceGate, loadEnrollment, useDeviceClient, type DeviceClient, type Enrollment} from '@/shared/device';
import {Card, Footnote, Icon, List, ListIcon, Mono, Notice, Overline, PrimaryButton, Screen, SectionHeading, StatusView, Title, TopBar} from '@/shared/ui';
import {CodeCard} from './code-card';
import {useMaintenanceCode} from './use-maintenance-code';

const header = <TopBar onBack={() => router.back()} title="Maintenance" />;

export function MaintenanceScreen({deviceId}: {deviceId: string}) {
  const handle = useDeviceClient(deviceId);
  const [e, setE] = useState<Enrollment | null>();
  useEffect(() => { loadEnrollment(deviceId).then(setE).catch(() => setE(null)); }, [deviceId]);
  return (
    <DeviceGate handle={handle}>
      {handle.state === 'ready' && e !== undefined && (e && e.maintenance !== 'none'
        ? <Maintenance client={handle.client} deviceId={deviceId} enrollment={e} maintenance={e.maintenance} />
        : <Screen header={header}><StatusView icon="maintenance" tone="warning" title="No maintenance codes yet"
            body="This phone was set up before maintenance codes existed, or the device could not provide them. Set the device up again over Bluetooth to get them.">
            <PrimaryButton title="Set up" icon="bluetooth" accessibilityLabel="Set it up again next to the device" onPress={() => router.replace('/')} />
          </StatusView></Screen>)}
    </DeviceGate>
  );
}

function Maintenance({client, deviceId, enrollment, maintenance}: {client: DeviceClient; deviceId: string; enrollment: Enrollment; maintenance: Exclude<Enrollment['maintenance'], 'none'>}) {
  const {view, unlock} = useMaintenanceCode(client, deviceId, maintenance);
  const [copied, setCopied] = useState(false);
  useEffect(() => { if (!copied) return; const t = setTimeout(() => setCopied(false), 1500); return () => clearTimeout(t); }, [copied]);
  const shown = view.state === 'shown';

  return (
    <Screen header={header} scroll footer={shown ? undefined
      : <PrimaryButton title="Show code" icon="lock" accessibilityLabel="Show the maintenance code after Face ID or the passcode"
          busy={view.state === 'unlocking'} onPress={() => void unlock()} />}>
      <Title>Maintenance access</Title>
      <Footnote>A computer on the same Wi-Fi asks for this code to open a short maintenance session on the device. Only give it to someone you trust to work on the device.</Footnote>
      {view.state === 'locked' && view.error ? <Notice tone="warning">{view.error}</Notice> : null}
      {shown && (
        <CodeCard code={view.code} secondsLeft={view.secondsLeft} source={view.source} copied={copied}
          onCopy={() => void Clipboard.setStringAsync(view.code).then(() => { setCopied(true); void Haptics.selectionAsync(); })} />
      )}
      <Card>
        <View className="flex-row items-center gap-2"><Icon name="lock" size={16} color="secondary" /><Overline>Device fingerprint</Overline></View>
        <Mono selectable>{shortFingerprint(enrollment.caSha256)}</Mono>
        <Footnote>The computer shows the same groups the first time it connects. If they differ, do not continue.</Footnote>
      </Card>
      <List.Section heading={<SectionHeading icon="info" label="How it works" />} footer="A code works once. Getting codes on another phone stops this phone’s codes.">
        <List.Row title="The computer asks for a code" leading={<ListIcon icon="desktop" />} />
        <List.Row title="Show it here and type it there" leading={<ListIcon icon="maintenance" />} />
        <List.Row title="The session ends by itself within an hour" leading={<ListIcon icon="check" />} />
      </List.Section>
    </Screen>
  );
}
