import {router} from 'expo-router';
// Opening the Studio on a PC. The PC has no BLE-delivered pin, so the user
// installs the device CA there and compares its fingerprint with the one
// this phone received sealed from the device. The download itself is not
// authenticated (anyone on the LAN can answer for the device's name), so
// that comparison is the only protection and the steps insist on it.
import {useEffect, useState} from 'react';
import {Share} from 'react-native';
import type {ManagementEnrollment} from '@stephania/provisioning-client';
import {formatFingerprint, loadEnrollment} from '@/shared/device';
import {Card, Icon, List, ListIcon, Mono, Notice, Overline, PrimaryButton, Screen, SectionHeading, StatusView, Title, TopBar} from '@/shared/ui';
import {View} from 'react-native';

export function PcAccessScreen({deviceId}: {deviceId: string}) {
  const [e, setE] = useState<ManagementEnrollment | null>();
  useEffect(() => { loadEnrollment(deviceId).then(setE).catch(() => setE(null)); }, [deviceId]);
  if (e === undefined) return <Screen header={<TopBar onBack={() => router.back()} title="Open on PC" />}><StatusView icon={'desktop'} title="Loading…" /></Screen>;
  if (e === null) return <Screen header={<TopBar onBack={() => router.back()} title="Open on PC" />}><StatusView icon={'warning'} tone="warning" title="No access" body="This phone has no access to this Stephania device. Set it up again from Wi-Fi setup." /></Screen>;
  const url = e.urls[0]!;
  const fingerprint = formatFingerprint(e.caSha256);
  const download = `${url}ca.crt`;
  return (
    <Screen header={<TopBar onBack={() => router.back()} title="Open on PC" />} scroll footer={<PrimaryButton title="Share" icon={'share'} accessibilityLabel="Share address and fingerprint"
      onPress={() => void Share.share({message: `${url}\nCertificate: ${download}\nSHA-256 (must match exactly before installing): ${fingerprint}`})} />}>
      <Title>Open the Studio on a PC</Title>
      <Card>
        <View className="flex-row items-center gap-2"><Icon name="desktop" size={16} color="secondary" /><Overline>Address</Overline></View>
        <Mono selectable>{url}</Mono>
        <View className="mt-1 flex-row items-center gap-2"><Icon name="lock" size={16} color="secondary" /><Overline>Certificate SHA-256</Overline></View>
        <Mono selectable>{fingerprint}</Mono>
      </Card>
      <Notice tone="warning">Anyone on the same network can pretend to be this device. Install the certificate only if all 64 characters of its SHA-256 match the fingerprint above.</Notice>
      <List.Section heading={<SectionHeading icon="info" label="Steps" />} footer="If the fingerprints differ, delete the file, do not install it, and check the network: another device may be impersonating Stephania.">
        <List.Row title="Join the same Wi-Fi on the PC" leading={<ListIcon icon="wifi" />} />
        <List.Row title={`Download ${download}`} detail="The browser warns that the connection is not private. That is expected for this download only; do not sign in on that page." leading={<ListIcon icon="desktop" />} />
        <List.Row title="Compute the file's SHA-256" detail="Windows: certutil -hashfile stephania-ca.crt SHA256 · macOS: shasum -a 256 stephania-ca.crt · Linux: sha256sum stephania-ca.crt" leading={<ListIcon icon="check" />} />
        <List.Row title="Compare it with the fingerprint above" detail="Every character must match. Letter case and spaces do not matter." leading={<ListIcon icon="check" />} />
        <List.Row title="Only then install it as a trusted root" leading={<ListIcon icon="lock" />} />
        <List.Row title="Open the address and sign in" leading={<ListIcon icon="arrow" />} />
      </List.Section>
    </Screen>
  );
}
