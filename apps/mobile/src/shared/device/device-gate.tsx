import {ActivityIndicator} from 'react-native';
import {router} from 'expo-router';
import type {ReactNode} from 'react';
import {PrimaryButton, Screen, StatusView, color, symbols} from '@/shared/ui';
import type {DeviceHandle} from './use-device-client';

/**
 * Loading / no-access states shared by every per-device screen. Renders
 * `children` only when the device is ready, so screens never handle a
 * missing client.
 */
export function DeviceGate({handle, children}: {handle: DeviceHandle; children: ReactNode}) {
  if (handle.state === 'loading') return <Screen><StatusView symbol={symbols.broadcast} title="Opening…"><ActivityIndicator color={color.textSecondary} /></StatusView></Screen>;
  if (handle.state === 'missing') {
    return <Screen><StatusView symbol={symbols.warning} tone="warning" title="No access to this T3Box" body="Set it up again over Bluetooth to give this phone access.">
      <PrimaryButton title="Find nearby T3Box" onPress={() => router.replace('/setup/nearby')} />
    </StatusView></Screen>;
  }
  return <>{children}</>;
}
