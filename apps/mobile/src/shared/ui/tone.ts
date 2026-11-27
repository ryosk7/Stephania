// State tones (DESIGN.md §3.1): colour means state, nothing else.
import type {ColorToken} from './theme';

export type Tone = 'neutral' | 'live' | 'warning' | 'success' | 'info';

/** Colour token per tone, for icons and dots. */
export const toneToken: Record<Tone, ColorToken> = {neutral: 'secondary', live: 'live', warning: 'warning', success: 'success', info: 'info'};

/** Same mapping as className fragments (static strings, so Tailwind sees every class). */
export const toneBg: Record<Tone, string> = {neutral: 'bg-secondary', live: 'bg-live', warning: 'bg-warning', success: 'bg-success', info: 'bg-info'};
export const toneBorder: Record<Tone, string> = {neutral: 'border-border', live: 'border-live', warning: 'border-warning', success: 'border-success', info: 'border-info'};
