// The Stephania device as drawn on the setup sheet: a Raspberry Pi 5 from
// above in its real colours (green board, gold GPIO pins and mounting rings,
// silver connectors with blue USB 3 and black USB 2, the active cooler's
// aluminium and black fan, the red power light).
// The same drawing is the AccessorySetupKit product image (iOS module).
import Svg, {Circle, G, Path, Rect} from 'react-native-svg';

export function DeviceArt({width = 150}: {width?: number}) {
  return (
    <Svg width={width} height={Math.round(width * 0.72)} viewBox="0 0 120 86" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <Rect x={4} y={8} width={102} height={70} rx={7} fill="#1F6B3F" />
      <Rect x={4.5} y={8.5} width={101} height={69} rx={6.5} fill="none" stroke="rgba(0,0,0,0.18)" />
      <G fill="#0E3A21" stroke="#D4AF37" strokeWidth={1.2}>
        <Circle cx={10.5} cy={14.5} r={2.4} /><Circle cx={68.5} cy={14.5} r={2.4} />
        <Circle cx={10.5} cy={71.5} r={2.4} /><Circle cx={68.5} cy={71.5} r={2.4} />
      </G>
      <Rect x={17} y={11} width={46} height={7} rx={1.5} fill="#18181B" />
      <Path d="M19.5 13h41.5M19.5 16h41.5" stroke="#D4AF37" strokeWidth={1.4} strokeDasharray="1.4 0.9" />
      <Rect x={18} y={26} width={34} height={34} rx={5} fill="#B9BEC4" />
      <Circle cx={35} cy={43} r={12} fill="#27272A" />
      <Circle cx={35} cy={43} r={3} fill="#52525B" />
      <Path d="M35 33.5v5M35 47.5v5M25.5 43h5M39.5 43h5M28.3 36.3l3.5 3.5M38.2 46.2l3.5 3.5M28.3 49.7l3.5-3.5M38.2 39.8l3.5-3.5"
        stroke="#52525B" strokeWidth={1.6} strokeLinecap="round" />
      <Rect x={58} y={30} width={16} height={16} rx={2} fill="#27272A" />
      <G fill="#C4C8CD"><Rect x={94} y={12} width={20} height={15} rx={1.5} /><Rect x={94} y={31} width={20} height={15} rx={1.5} /></G>
      <Rect x={99} y={17} width={12} height={2.5} rx={0.6} fill="#18181B" />
      <Rect x={99} y={36} width={12} height={2.5} rx={0.6} fill="#2563EB" />
      <Rect x={92} y={51} width={22} height={20} rx={1.5} fill="#C4C8CD" />
      <Rect x={98} y={56} width={12} height={10} rx={1} fill="#3F3F46" />
      <G fill="#C4C8CD"><Rect x={16} y={74} width={10} height={6} rx={2} /><Rect x={32} y={74} width={9} height={6} rx={1.5} /><Rect x={46} y={74} width={9} height={6} rx={1.5} /></G>
      <Circle cx={84} cy={70} r={2.6} fill="#F04438" />
    </Svg>
  );
}
