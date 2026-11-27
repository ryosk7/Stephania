// Opening the Studio on a PC. The PC has no BLE-delivered pin, so the user
// installs the device CA there and compares its fingerprint with the one
// this phone received sealed from the device.
import {useLocalSearchParams} from 'expo-router';
import {useEffect, useState} from 'react';
import {Share, Text} from 'react-native';
import type {ManagementEnrollment} from '@t3box/provisioning-client';
import {formatFingerprint} from '../net/device';
import {loadEnrollment} from '../state/session';
import {Button, Screen, s} from '../ui';

export default function OpenOnPc() {
  const {device} = useLocalSearchParams<{device: string}>();
  const [e, setE] = useState<ManagementEnrollment | null>();
  useEffect(() => { loadEnrollment(device).then(setE).catch(() => setE(null)); }, [device]);
  if (e === undefined) return <Screen><Text style={s.p}>Loading…</Text></Screen>;
  if (e === null) return <Screen><Text style={s.p}>This phone has no access to this T3Box.</Text></Screen>;
  const url = e.urls[0]!;
  return (
    <Screen>
      <Text style={s.p}>Open this address on the PC (on the same Wi-Fi network).</Text>
      <Text selectable style={s.h}>{url}</Text>
      <Text style={s.p}>
        The browser warns about the certificate the first time. Install the T3Box certificate ({url}ca.pem) on the PC as a trusted root certificate,
        after checking that its fingerprint matches the value below. If it does not match, do not install it and check the network.
      </Text>
      <Text selectable style={[s.p, {fontFamily: 'Menlo', fontWeight: '600'}]}>SHA-256{'\n'}{formatFingerprint(e.caSha256)}</Text>
      <Text style={s.p}>Sign in with a Studio account (created by an admin).</Text>
      <Button title="Share address and fingerprint" onPress={() => void Share.share({message: `${url}\nSHA-256: ${formatFingerprint(e.caSha256)}`})} />
    </Screen>
  );
}
