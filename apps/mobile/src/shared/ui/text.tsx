// One component per iOS text style (DESIGN.md §4): explicit variants, no
// `size`/`bold` flags.
import type {ReactNode} from 'react';
import {Text, type TextProps} from 'react-native';
import {color, toneColor, type, type Tone} from './tokens';

type Props = TextProps & {children: ReactNode; tone?: Tone | 'primary' | 'secondary' | 'tertiary'};

const tint = (tone: Props['tone']) =>
  tone === undefined || tone === 'primary' ? color.text
    : tone === 'secondary' ? color.textSecondary
      : tone === 'tertiary' ? color.textTertiary
        : toneColor[tone];

function make(style: (typeof type)[keyof typeof type], defaultTone: Props['tone'], header = false) {
  return function StyledText({tone = defaultTone, style: extra, ...rest}: Props) {
    return <Text accessibilityRole={header ? 'header' : undefined} {...rest} style={[style, {color: tint(tone)}, extra]} />;
  };
}

export const LargeTitle = make(type.largeTitle, 'primary', true);
export const Title = make(type.title, 'primary', true);
export const Headline = make(type.headline, 'primary');
export const Body = make(type.body, 'primary');
export const Callout = make(type.callout, 'secondary');
export const Subheadline = make(type.subheadline, 'secondary');
export const Footnote = make(type.footnote, 'secondary');
export const Caption = make(type.caption, 'secondary');
export const Mono = make(type.mono, 'primary');
