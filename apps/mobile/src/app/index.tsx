import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {Text} from 'react-native';
import {enrolledDevices, type EnrolledDevice} from '../state/session';
import {Button, Screen, s} from '../ui';

export default function Home() {
  const [enrolled, setEnrolled] = useState<EnrolledDevice[]>([]);
  useEffect(() => { enrolledDevices().then(setEnrolled).catch(() => undefined); }, []);
  return (
    <Screen>
      <Text style={s.h}>Get started with T3Box</Text>
      <Text style={s.p}>
        1. On a PC, open the setup page and press “Create secret”{'\n'}
        2. Copy t3box-setup.txt to the boot partition of the microSD card with the OS{'\n'}
        3. Insert the card into the T3Box and power it on{'\n'}
        4. Scan the QR code on the PC screen with the button below
      </Text>
      <Button title="Scan setup QR" onPress={() => router.push('/scan')} />
      <Button title="Connect to a registered T3Box" onPress={() => router.push('/devices')} />
      {enrolled.length > 0 && <Text style={s.h}>Control over Wi-Fi</Text>}
      {enrolled.map(d => (
        <Button key={d.deviceId} title={`Control ${d.name}`} onPress={() => router.push({pathname: '/dashboard', params: {device: d.deviceId}})} />
      ))}
    </Screen>
  );
}
