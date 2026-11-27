// One icon set (Lucide, ISC) drawn the same on every platform, 2 px strokes
// on a 24 grid. Screens name icons from `icons`, never a platform set.
import {
  Bluetooth, Check, ChevronLeft, ChevronRight, CircleCheck, Lock, Mic, Monitor, Pencil, Plus, Presentation, QrCode,
  Radio, Share, Thermometer, TriangleExclamationPoint, User, Video, Volume2, Wifi, WifiHigh, WifiLow, X,
} from 'lucide-react-native';
import {color, toneColor, type Tone} from './tokens';

export const icons = {
  close: X, back: ChevronLeft, chevron: ChevronRight, qr: QrCode, bluetooth: Bluetooth, wifi: Wifi, wifiMid: WifiHigh, wifiLow: WifiLow,
  lock: Lock, done: CircleCheck, check: Check, warning: TriangleExclamationPoint, thermometer: Thermometer, camera: Video,
  broadcast: Radio, desktop: Monitor, speaker: Volume2, mic: Mic, share: Share, plus: Plus, pencil: Pencil, person: User, slides: Presentation,
};
export type IconName = keyof typeof icons;

export function Icon({name, size = 20, tone, tint, strokeWidth = 2}: {name: IconName; size?: number; tone?: Tone; tint?: string; strokeWidth?: number}) {
  const C = icons[name];
  return <C size={size} color={tint ?? (tone ? toneColor[tone] : color.text)} strokeWidth={strokeWidth} accessibilityElementsHidden importantForAccessibility="no" />;
}
