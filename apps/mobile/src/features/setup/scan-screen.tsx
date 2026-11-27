// Setup step 1: read the setup QR from the PC screen (pairing pattern:
// dark viewfinder with corner brackets, instruction, quiet way out).
import {CameraView, useCameraPermissions} from 'expo-camera';
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useRef, useState} from 'react';
import {StyleSheet, View} from 'react-native';
import {parseSetupUri} from '@stephania/provisioning-client';
import {Body, Icon, Notice, PrimaryButton, Screen, SecondaryButton, StatusView, TopBar} from '@/shared/ui';
import {setPendingSecret} from './state';

export function ScanScreen() {
  const [permission, requestPermission] = useCameraPermissions();
  const [error, setError] = useState('');
  const handled = useRef(false);

  if (!permission) return <Screen header={<TopBar onBack={() => router.back()} title="Set up" />}><StatusView icon={'qr'} title="Preparing the camera…" /></Screen>;
  if (!permission.granted) {
    return (
      <Screen header={<TopBar onBack={() => router.back()} title="Set up" />} footer={<PrimaryButton title="Allow camera" icon="camera" onPress={() => void requestPermission()} />}>
        <StatusView icon={'qr'} title="Scan the code on your PC" body="The camera reads the QR code on the Stephania setup page." />
      </Screen>
    );
  }
  return (
    <Screen header={<TopBar onBack={() => router.back()} title="Set up" />} footer={<SecondaryButton title="Nearby" icon="bluetooth" accessibilityLabel="Find it nearby instead" onPress={() => router.replace('/setup/nearby')} />}>
      <View className="mt-6 aspect-square overflow-hidden rounded-[32px] bg-video">
        <CameraView style={StyleSheet.absoluteFill} facing="back" barcodeScannerSettings={{barcodeTypes: ['qr']}}
          onBarcodeScanned={({data}) => {
            if (handled.current) return;
            const secret = parseSetupUri(data);
            if (!secret) { setError('This is not a Stephania setup code.'); return; }
            handled.current = true;
            void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
            setPendingSecret(secret);
            router.replace('/setup/nearby');
          }} />
        <Brackets />
      </View>
      <View className="flex-row items-center justify-center gap-3.5" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        <Icon name="desktop" size={28} color="secondary" />
        <Icon name="arrow" size={20} color="tertiary" />
        <Icon name="qr" size={28} />
      </View>
      <Body className="text-center font-inter-medium">Scan the code on your PC</Body>
      {error !== '' && <Notice tone="warning">{error}</Notice>}
    </Screen>
  );
}

/** Corner brackets over the viewfinder (white: the viewfinder is video, dark in both themes). */
function Brackets() {
  const c = 'absolute h-11 w-11 border-white';
  return (
    <View style={StyleSheet.absoluteFill} pointerEvents="none">
      <View className={`${c} left-6 top-6 rounded-tl-tile border-l-4 border-t-4`} />
      <View className={`${c} right-6 top-6 rounded-tr-tile border-r-4 border-t-4`} />
      <View className={`${c} bottom-6 left-6 rounded-bl-tile border-b-4 border-l-4`} />
      <View className={`${c} bottom-6 right-6 rounded-br-tile border-b-4 border-r-4`} />
    </View>
  );
}
