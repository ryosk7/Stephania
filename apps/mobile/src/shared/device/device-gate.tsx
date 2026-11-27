import {ActivityIndicator} from 'react-native';
import {router} from 'expo-router';
import type {ReactNode} from 'react';
import {PrimaryButton, Screen, StatusView, TopBar, useColor} from '@/shared/ui';
import type {DeviceHandle} from './use-device-client';

/**
 * Loading / no-access states shared by every per-device screen. Renders
 * `children` only when the device is ready, so screens never handle a
 * missing client.
 */
export function DeviceGate({handle, children}: {handle: DeviceHandle; children: ReactNode}) {
  const spinner = useColor('secondary');
  if (handle.state === 'loading') return <Screen header={<TopBar onBack={() => router.back()} />}><StatusView icon={'broadcast'} title="Opening…"><ActivityIndicator color={spinner} /></StatusView></Screen>;
  if (handle.state === 'missing') {
    return <Screen header={<TopBar onBack={() => router.back()} />}><StatusView icon={'warning'} tone="warning" title="No access to this Stephania device" body="Set it up again over Bluetooth to give this phone access.">
      <PrimaryButton title="Set up" icon="bluetooth" accessibilityLabel="Set it up again next to the device" onPress={() => router.replace('/')} />
    </StatusView></Screen>;
  }
  return <>{children}</>;
}
