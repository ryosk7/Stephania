// The maintenance code as shown to the user: six large digits, a ring
// counting down the 30 s step, tap to copy. Presentational only (story:
// code-card.stories.tsx); the screen computes the code.
import {Pressable, Text, View} from 'react-native';
import Svg, {Circle} from 'react-native-svg';
import {TOTP_STEP_SECONDS} from '@stephania/provisioning-client';
import {Card, Footnote, Icon, Overline, useColor} from '@/shared/ui';

export type ClockSource = 'device' | 'phone';

const RING = 44;
const STROKE = 4;
const R = (RING - STROKE) / 2;
const C = 2 * Math.PI * R;

export function CountdownRing({secondsLeft}: {secondsLeft: number}) {
  const track = useColor('border');
  const fg = useColor(secondsLeft <= 5 ? 'warning' : 'text');
  const left = Math.max(0, Math.min(TOTP_STEP_SECONDS, secondsLeft));
  return (
    <View accessible accessibilityLabel={`${Math.ceil(left)} seconds left`} className="items-center justify-center" style={{width: RING, height: RING}}>
      <Svg width={RING} height={RING} style={{position: 'absolute', transform: [{rotate: '-90deg'}]}}>
        <Circle cx={RING / 2} cy={RING / 2} r={R} stroke={track} strokeWidth={STROKE} fill="none" />
        <Circle cx={RING / 2} cy={RING / 2} r={R} stroke={fg} strokeWidth={STROKE} fill="none" strokeLinecap="round"
          strokeDasharray={`${C} ${C}`} strokeDashoffset={C * (1 - left / TOTP_STEP_SECONDS)} />
      </Svg>
      <Text className="font-mono text-footnote text-secondary">{Math.ceil(left)}</Text>
    </View>
  );
}

export function CodeCard({code, secondsLeft, source, copied, onCopy}: {
  code: string; secondsLeft: number; source: ClockSource; copied?: boolean; onCopy?: () => void;
}) {
  const spoken = code.split('').join(' ');
  return (
    <Card>
      <View className="flex-row items-center gap-2">
        <Icon name="maintenance" size={16} color="secondary" />
        <Overline>Maintenance code</Overline>
      </View>
      <Pressable accessibilityRole="button" accessibilityLabel={`Code ${spoken}. Copy`} onPress={onCopy}
        className="flex-row items-center justify-between gap-3 active:opacity-70">
        <Text selectable className="font-mono text-text" style={{fontSize: 40, lineHeight: 48, letterSpacing: 4}}>
          {code.slice(0, 3)} {code.slice(3)}
        </Text>
        <CountdownRing secondsLeft={secondsLeft} />
      </Pressable>
      <View className="flex-row items-center gap-1.5">
        <Icon name={copied ? 'check' : 'copy'} size={14} color="secondary" />
        <Footnote>{copied ? 'Copied' : 'Tap to copy'}</Footnote>
      </View>
      {source === 'phone' && (
        <Footnote tone="warning">Using this phone’s clock: the device could not be reached to check its time. If the code is refused, open this screen on the same Wi-Fi as the device.</Footnote>
      )}
    </Card>
  );
}
