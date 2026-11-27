// Design tokens (DESIGN.md §4). T3Box's own palette, identical on iOS and
// Android: nothing here follows one platform's system colours.
import type {TextStyle} from 'react-native';

export const color = {
  bg: '#0B0B0C',
  surface: '#161618',
  surfaceRaised: '#1F1F23',
  /** Control backgrounds: secondary buttons, icon buttons, pills, tracks. */
  fill: 'rgba(255,255,255,0.08)',
  border: 'rgba(255,255,255,0.08)',
  separator: 'rgba(255,255,255,0.08)',
  text: '#F4F4F5',
  textSecondary: '#A1A1AA',
  textTertiary: '#71717A',
  live: '#F04438',
  warning: '#F5B300',
  success: '#22C55E',
  info: '#3B82F6',
  onPrimary: '#0B0B0C',
  scrim: 'rgba(0,0,0,0.6)',
} as const;

export type Tone = 'neutral' | 'live' | 'warning' | 'success' | 'info';

export const toneColor: Record<Tone, string> = {
  neutral: color.textSecondary,
  live: color.live,
  warning: color.warning,
  success: color.success,
  info: color.info,
};

export const space = {xs: 4, s: 8, m: 12, l: 16, xl: 24, xxl: 32} as const;
export const radius = {card: 16, control: 12, button: 14, tile: 12, sheet: 24, pill: 999} as const;
export const size = {hit: 44, button: 52, screenPadding: 16} as const;

/** Bundled fonts (loaded in the root layout), so text is the same on every platform. */
export const font = {
  regular: 'Inter_400Regular',
  medium: 'Inter_500Medium',
  semibold: 'Inter_600SemiBold',
  bold: 'Inter_700Bold',
  mono: 'JetBrainsMono_500Medium',
} as const;

// Custom fonts carry their weight in the family name; fontWeight stays unset
// so Android does not synthesise a second weight.
export const type = {
  display: {fontFamily: font.bold, fontSize: 32, lineHeight: 38, letterSpacing: -0.6},
  title: {fontFamily: font.semibold, fontSize: 22, lineHeight: 28, letterSpacing: -0.3},
  headline: {fontFamily: font.semibold, fontSize: 16, lineHeight: 22, letterSpacing: -0.1},
  body: {fontFamily: font.regular, fontSize: 16, lineHeight: 22},
  label: {fontFamily: font.medium, fontSize: 14, lineHeight: 20},
  footnote: {fontFamily: font.regular, fontSize: 13, lineHeight: 18},
  overline: {fontFamily: font.semibold, fontSize: 11, lineHeight: 14, letterSpacing: 0.8, textTransform: 'uppercase'},
  mono: {fontFamily: font.mono, fontSize: 13, lineHeight: 18},
} as const satisfies Record<string, TextStyle>;
