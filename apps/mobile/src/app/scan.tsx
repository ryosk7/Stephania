import {CameraView, useCameraPermissions} from 'expo-camera';
import {router} from 'expo-router';
import {useRef, useState} from 'react';
import {Text, View} from 'react-native';
import {parseSetupUri} from '@t3box/provisioning-client';
import {setPendingSecret} from '../state/session';
import {Button, Screen, s} from '../ui';

export default function Scan() {
  const [permission, requestPermission] = useCameraPermissions();
  const [error, setError] = useState('');
  const handled = useRef(false);

  if (!permission) return <Screen><Text style={s.p}>Preparing the camera…</Text></Screen>;
  if (!permission.granted) {
    return (
      <Screen>
        <Text style={s.p}>The camera is used to read the setup QR code.</Text>
        <Button title="Allow camera" onPress={() => void requestPermission()} />
      </Screen>
    );
  }
  return (
    <View style={{flex: 1}}>
      <CameraView
        style={{flex: 1}}
        facing="back"
        barcodeScannerSettings={{barcodeTypes: ['qr']}}
        onBarcodeScanned={({data}) => {
          if (handled.current) return;
          const secret = parseSetupUri(data);
          if (!secret) { setError('This is not a T3Box setup QR code.'); return; }
          handled.current = true;
          setPendingSecret(secret);
          router.replace('/devices');
        }}
      />
      {error !== '' && <Text style={[s.error, {padding: 16, backgroundColor: '#fff'}]}>{error}</Text>}
    </View>
  );
}
