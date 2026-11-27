// One icon set (Lucide, ISC) drawn the same on every platform, 2 px strokes
// on a 24 grid. Icons carry meaning first (DESIGN.md §3.7): screens name
// them from `icons`, and every icon-only control has a spoken label.
import {
  ArrowRight, Blend, Bluetooth, Cable, Check, ChevronLeft, ChevronRight, CircleCheck, Info, LayoutGrid, Lock, Mic, MicOff, Monitor,
  Moon, Pencil, PictureInPicture2, Plus, Presentation, Radio, RefreshCw, ScanFace, Settings, Share, SignalHigh, SignalLow, SignalMedium, SignalZero, SlidersHorizontal, Square,
  Sun, SunMoon, Thermometer, TriangleExclamationPoint, User, Video, Volume2, VolumeX, Wifi, WifiOff, X,
} from 'lucide-react-native';
import {useColor, type ColorToken} from './theme';
import {toneToken, type Tone} from './tone';

export const icons = {
  close: X, back: ChevronLeft, chevron: ChevronRight, arrow: ArrowRight, bluetooth: Bluetooth, wifi: Wifi, wifiOff: WifiOff,
  lock: Lock, done: CircleCheck, check: Check, warning: TriangleExclamationPoint, info: Info, thermometer: Thermometer,
  camera: Video, selfie: ScanFace, bothCameras: PictureInPicture2, broadcast: Radio, stop: Square, desktop: Monitor,
  speaker: Volume2, speakerOff: VolumeX, mic: Mic, micOff: MicOff, share: Share, plus: Plus, pencil: Pencil, person: User, slides: Presentation,
  layouts: LayoutGrid, fade: Blend, control: SlidersHorizontal, settings: Settings, refresh: RefreshCw,
  signal0: SignalZero, signal1: SignalLow, signal2: SignalMedium, signal3: SignalHigh, cable: Cable,
  system: SunMoon, light: Sun, dark: Moon,
};
export type IconName = keyof typeof icons;

/** `tone` for state colours, `color` for any other token; defaults to the text colour. */
export function Icon({name, size = 20, tone, color, strokeWidth = 2, fill}: {
  name: IconName; size?: number; tone?: Tone; color?: ColorToken; strokeWidth?: number; fill?: ColorToken;
}) {
  const C = icons[name];
  const stroke = useColor(color ?? (tone ? toneToken[tone] : 'text'));
  const fillColor = useColor(fill ?? color ?? (tone ? toneToken[tone] : 'text'));
  // Stop reads as the media-control square only when solid.
  const solid = fill !== undefined || name === 'stop';
  return <C size={size} color={stroke} fill={solid ? fillColor : 'none'} strokeWidth={strokeWidth} accessibilityElementsHidden importantForAccessibility="no" />;
}
