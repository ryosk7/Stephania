// Opening the Studio on a PC. The PC has no BLE-delivered pin, so the user
// installs the device CA there and compares its fingerprint with the one
// this phone received sealed from the device.
import {useEffect, useState} from 'react';
import {Share} from 'react-native';
import type {ManagementEnrollment} from '@t3box/provisioning-client';
import {formatFingerprint, loadEnrollment} from '@/shared/device';
import {Caption, Card, List, Mono, PrimaryButton, Screen, StatusView, Title, symbols} from '@/shared/ui';

export function PcAccessScreen({deviceId}: {deviceId: string}) {
  const [e, setE] = useState<ManagementEnrollment | null>();
  useEffect(() => { loadEnrollment(deviceId).then(setE).catch(() => setE(null)); }, [deviceId]);
  if (e === undefined) return <Screen><StatusView symbol={symbols.desktop} title="Loading…" /></Screen>;
  if (e === null) return <Screen><StatusView symbol={symbols.warning} tone="warning" title="No access" body="This phone has no access to this T3Box. Set it up again from Wi-Fi setup." /></Screen>;
  const url = e.urls[0]!;
  const fingerprint = formatFingerprint(e.caSha256);
  return (
    <Screen scroll footer={<PrimaryButton title="Share address and fingerprint" symbol={symbols.share}
      onPress={() => void Share.share({message: `${url}\nSHA-256: ${fingerprint}`})} />}>
      <Title>Open the Studio on a PC</Title>
      <Card>
        <Caption>ADDRESS</Caption>
        <Mono selectable>{url}</Mono>
        <Caption>CERTIFICATE SHA-256</Caption>
        <Mono selectable>{fingerprint}</Mono>
      </Card>
      <List.Section title="Steps" footer="If the fingerprints differ, do not install the certificate and check the network.">
        <List.Row title="1. Join the same Wi-Fi on the PC" />
        <List.Row title={`2. Download ${url}ca.pem`} />
        <List.Row title="3. Check that its fingerprint matches the one above" />
        <List.Row title="4. Install it as a trusted root certificate" />
        <List.Row title="5. Open the address and sign in with a Studio account" />
      </List.Section>
    </Screen>
  );
}
