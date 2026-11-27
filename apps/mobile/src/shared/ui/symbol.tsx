// SF Symbols on iOS, Material Symbols on Android (expo-symbols). Names are
// given for both platforms so the Android port does not touch screens.
import {SymbolView, type SymbolViewProps} from 'expo-symbols';
import {color, toneColor, type Tone} from './tokens';

type PlatformNames = Exclude<SymbolViewProps['name'], string>;
/** Both names required; typed against SF Symbols / Material Symbols, so a typo fails typecheck. */
export type SymbolName = Required<Pick<PlatformNames, 'ios' | 'android'>>;

/** Symbols used in the app, one place to keep both platforms' names. */
export const symbols = {
  close: {ios: 'xmark', android: 'close'},
  back: {ios: 'chevron.left', android: 'arrow_back'},
  chevron: {ios: 'chevron.right', android: 'chevron_right'},
  qr: {ios: 'qrcode.viewfinder', android: 'qr_code_scanner'},
  bluetooth: {ios: 'antenna.radiowaves.left.and.right', android: 'bluetooth_searching'},
  wifi: {ios: 'wifi', android: 'wifi'},
  lock: {ios: 'lock.fill', android: 'lock'},
  check: {ios: 'checkmark.circle.fill', android: 'check_circle'},
  warning: {ios: 'exclamationmark.triangle.fill', android: 'warning'},
  thermometer: {ios: 'thermometer.medium', android: 'device_thermostat'},
  camera: {ios: 'video.fill', android: 'videocam'},
  broadcast: {ios: 'dot.radiowaves.left.and.right', android: 'sensors'},
  desktop: {ios: 'desktopcomputer', android: 'computer'},
  speaker: {ios: 'speaker.wave.2.fill', android: 'volume_up'},
  mic: {ios: 'mic.fill', android: 'mic'},
  share: {ios: 'square.and.arrow.up', android: 'share'},
  plus: {ios: 'plus', android: 'add'},
  pencil: {ios: 'pencil', android: 'edit'},
  person: {ios: 'person.fill', android: 'person'},
  slides: {ios: 'display', android: 'present_to_all'},
} satisfies Record<string, SymbolName>;

export function Symbol({name, size = 20, tone, tint}: {name: SymbolName; size?: number; tone?: Tone; tint?: string}) {
  return <SymbolView name={name} size={size} tintColor={tint ?? (tone ? toneColor[tone] : color.text)} />;
}
