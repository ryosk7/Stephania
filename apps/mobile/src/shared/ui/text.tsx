// One component per text style (DESIGN.md §4): explicit variants, no
// `size`/`bold` flags. All Inter, so iOS and Android render alike.
import type {ReactNode} from 'react';
import {Text, type TextProps} from 'react-native';

type TextTone = 'primary' | 'secondary' | 'tertiary' | 'live' | 'warning' | 'success' | 'info';
type Props = TextProps & {children: ReactNode; tone?: TextTone; className?: string};

const toneClass: Record<TextTone, string> = {
  primary: 'text-text', secondary: 'text-secondary', tertiary: 'text-tertiary',
  live: 'text-live', warning: 'text-warning', success: 'text-success', info: 'text-info',
};

function make(variant: string, defaultTone: TextTone, header = false) {
  return function StyledText({tone = defaultTone, className = '', ...rest}: Props) {
    return <Text accessibilityRole={header ? 'header' : undefined} {...rest} className={`${variant} ${toneClass[tone]} ${className}`} />;
  };
}

export const Display = make('font-inter-bold text-display', 'primary', true);
export const Title = make('font-inter-semibold text-title', 'primary', true);
export const Headline = make('font-inter-semibold text-headline', 'primary');
export const Body = make('font-inter text-body', 'primary');
export const Label = make('font-inter-medium text-label', 'primary');
export const Footnote = make('font-inter text-footnote', 'secondary');
/** Small uppercase label (tags, captions); section headings use SectionHeading's icon. */
export const Overline = make('font-inter-semibold text-overline uppercase', 'secondary', true);
/** Addresses, fingerprints, numbers that must line up. */
export const Mono = make('font-mono text-footnote', 'primary');
