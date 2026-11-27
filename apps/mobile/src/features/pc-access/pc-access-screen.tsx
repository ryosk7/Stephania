import {router} from 'expo-router';
// Opening the Studio on a PC. The PC has no BLE-delivered pin, so the user
// installs the device CA there and compares its fingerprint with the one
// this phone received sealed from the device.
import {useEffect, useState} from 'react';
import {Share} from 'react-native';
import type {ManagementEnrollment} from '@t3box/provisioning-client';
import {formatFingerprint, loadEnrollment} from '@/shared/device';
import {Card, Icon, List, ListIcon, Mono, Overline, PrimaryButton, Screen, SectionHeading, StatusView, Title, TopBar} from '@/shared/ui';
import {View} from 'react-native';

export function PcAccessScreen({deviceId}: {deviceId: string}) {
  const [e, setE] = useState<ManagementEnrollment | null>();
  useEffect(() => { loadEnrollment(deviceId).then(setE).catch(() => setE(null)); }, [deviceId]);
  if (e === undefined) return <Screen header={<TopBar onBack={() => router.back()} title="Open on PC" />}><StatusView icon={'desktop'} title="Loading…" /></Screen>;
  if (e === null) return <Screen header={<TopBar onBack={() => router.back()} title="Open on PC" />}><StatusView icon={'warning'} tone="warning" title="No access" body="This phone has no access to this T3Box. Set it up again from Wi-Fi setup." /></Screen>;
  const url = e.urls[0]!;
  const fingerprint = formatFingerprint(e.caSha256);
  return (
    <Screen header={<TopBar onBack={() => router.back()} title="Open on PC" />} scroll footer={<PrimaryButton title="Share" icon={'share'} accessibilityLabel="Share address and fingerprint"
      onPress={() => void Share.share({message: `${url}\nSHA-256: ${fingerprint}`})} />}>
      <Title>Open the Studio on a PC</Title>
      <Card>
        <View className="flex-row items-center gap-2"><Icon name="desktop" size={16} color="secondary" /><Overline>Address</Overline></View>
        <Mono selectable>{url}</Mono>
        <View className="mt-1 flex-row items-center gap-2"><Icon name="lock" size={16} color="secondary" /><Overline>Certificate SHA-256</Overline></View>
        <Mono selectable>{fingerprint}</Mono>
      </Card>
      <List.Section heading={<SectionHeading icon="info" label="Steps" />} footer="If the fingerprints differ, do not install the certificate and check the network.">
        <List.Row title="Join the same Wi-Fi on the PC" leading={<ListIcon icon="wifi" />} />
        <List.Row title={`Download ${url}ca.pem`} leading={<ListIcon icon="desktop" />} />
        <List.Row title="Check the fingerprint matches" leading={<ListIcon icon="check" />} />
        <List.Row title="Install it as a trusted root" leading={<ListIcon icon="lock" />} />
        <List.Row title="Open the address and sign in" leading={<ListIcon icon="arrow" />} />
      </List.Section>
    </Screen>
  );
}
