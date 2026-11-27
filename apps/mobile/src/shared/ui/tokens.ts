// Design tokens (DESIGN.md §4). iOS dark-appearance system colours with
// semantic names, so another platform maps names, not screens.
export const color = {
  bg: '#000000',
  surface: '#1C1C1E',
  surfaceRaised: '#2C2C2E',
  separator: '#38383A',
  text: '#FFFFFF',
  textSecondary: 'rgba(235,235,245,0.6)',
  textTertiary: 'rgba(235,235,245,0.3)',
  live: '#FF453A',
  warning: '#FFD60A',
  success: '#30D158',
  info: '#0A84FF',
  onPrimary: '#000000',
  scrim: 'rgba(0,0,0,0.55)',
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
export const radius = {card: 16, tile: 12, control: 10, pill: 999} as const;
export const size = {hit: 44, primaryButton: 54, screenPadding: 16} as const;

export const type = {
  largeTitle: {fontSize: 34, lineHeight: 41, fontWeight: '700'},
  title: {fontSize: 22, lineHeight: 28, fontWeight: '700'},
  headline: {fontSize: 17, lineHeight: 22, fontWeight: '600'},
  body: {fontSize: 17, lineHeight: 22, fontWeight: '400'},
  callout: {fontSize: 16, lineHeight: 21, fontWeight: '400'},
  footnote: {fontSize: 13, lineHeight: 18, fontWeight: '400'},
  caption: {fontSize: 12, lineHeight: 16, fontWeight: '500'},
  mono: {fontSize: 13, lineHeight: 18, fontFamily: 'Menlo', fontWeight: '500'},
} as const;
