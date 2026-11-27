// One-tap setup through Apple's AccessorySetupKit (iOS 18+): the system
// sheet finds a nearby Stephania and grants Bluetooth access to that one
// accessory, with no Bluetooth permission prompt. Everywhere else
// `supported` is false and the app draws the same sheet itself.
import {requireOptionalNativeModule} from 'expo';

export type PickedAccessory = {
  /** CoreBluetooth peripheral identifier; react-native-ble-plx connects to it directly. */
  id: string;
  name: string;
};

type Native = {
  isSupported(): boolean;
  pick(serviceUuid: string, name: string): Promise<PickedAccessory | null>;
};

const native = requireOptionalNativeModule<Native>('StephaniaAccessorySetup');

export const accessorySetup = {
  supported: native?.isSupported() ?? false,
  /** Shows the system sheet; resolves null when the person dismisses it. */
  pick: (serviceUuid: string, name: string): Promise<PickedAccessory | null> =>
    native ? native.pick(serviceUuid, name) : Promise.resolve(null),
};
