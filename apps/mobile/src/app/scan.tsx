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

  if (!permission) return <Screen><Text style={s.p}>カメラを準備しています…</Text></Screen>;
  if (!permission.granted) {
    return (
      <Screen>
        <Text style={s.p}>セットアップQRを読むためにカメラを使います。</Text>
        <Button title="カメラを許可" onPress={() => void requestPermission()} />
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
          if (!secret) { setError('T3BoxのセットアップQRではありません。'); return; }
          handled.current = true;
          setPendingSecret(secret);
          router.replace('/devices');
        }}
      />
      {error !== '' && <Text style={[s.error, {padding: 16, backgroundColor: '#fff'}]}>{error}</Text>}
    </View>
  );
}
