// Setup step 1: read the setup QR from the PC screen (pairing pattern:
// dark viewfinder with corner brackets, instruction, quiet way out).
import {CameraView, useCameraPermissions} from 'expo-camera';
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useRef, useState} from 'react';
import {StyleSheet, View} from 'react-native';
import {parseSetupUri} from '@t3box/provisioning-client';
import {Body, Notice, PrimaryButton, Screen, StatusView, TextButton, color, radius, space, symbols} from '@/shared/ui';
import {setPendingSecret} from './state';

export function ScanScreen() {
  const [permission, requestPermission] = useCameraPermissions();
  const [error, setError] = useState('');
  const handled = useRef(false);

  if (!permission) return <Screen><StatusView symbol={symbols.qr} title="Preparing the camera…" /></Screen>;
  if (!permission.granted) {
    return (
      <Screen footer={<PrimaryButton title="Allow camera" onPress={() => void requestPermission()} />}>
        <StatusView symbol={symbols.qr} title="Scan the setup code" body="The camera reads the QR code shown by the T3Box setup page on your PC." />
      </Screen>
    );
  }
  return (
    <Screen footer={<TextButton title="Find it nearby instead" onPress={() => router.replace('/setup/nearby')} />}>
      <View style={st.finder}>
        <CameraView style={StyleSheet.absoluteFill} facing="back" barcodeScannerSettings={{barcodeTypes: ['qr']}}
          onBarcodeScanned={({data}) => {
            if (handled.current) return;
            const secret = parseSetupUri(data);
            if (!secret) { setError('This is not a T3Box setup code.'); return; }
            handled.current = true;
            void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
            setPendingSecret(secret);
            router.replace('/setup/nearby');
          }} />
        <Brackets />
      </View>
      <Body style={{textAlign: 'center'}}>Scan the QR code on the T3Box setup page</Body>
      {error !== '' && <Notice tone="warning">{error}</Notice>}
    </Screen>
  );
}

/** Corner brackets over the viewfinder. */
function Brackets() {
  const corner = (pos: object) => <View style={[st.corner, pos]} />;
  return (
    <View style={StyleSheet.absoluteFill} pointerEvents="none">
      {corner({top: space.xl, left: space.xl, borderTopWidth: 4, borderLeftWidth: 4, borderTopLeftRadius: radius.tile})}
      {corner({top: space.xl, right: space.xl, borderTopWidth: 4, borderRightWidth: 4, borderTopRightRadius: radius.tile})}
      {corner({bottom: space.xl, left: space.xl, borderBottomWidth: 4, borderLeftWidth: 4, borderBottomLeftRadius: radius.tile})}
      {corner({bottom: space.xl, right: space.xl, borderBottomWidth: 4, borderRightWidth: 4, borderBottomRightRadius: radius.tile})}
    </View>
  );
}

const st = StyleSheet.create({
  finder: {aspectRatio: 1, borderRadius: radius.card * 2, overflow: 'hidden', backgroundColor: color.surface, marginTop: space.xl},
  corner: {position: 'absolute', width: 44, height: 44, borderColor: color.text},
});
