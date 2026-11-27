import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import {Field, Footnote, PrimaryButton, SheetCard, Title} from '@/shared/ui';
import {DeviceArt} from './device-art';

const noop = () => {};
const meta = {title: 'Setup/Sheets', component: DeviceArt} satisfies Meta<typeof DeviceArt>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Device: Story = {render: () => <View className="items-center"><DeviceArt /></View>};

/** One device: picture, name, one action (the iOS 18 system sheet shows the same picture). */
export const OneDevice: Story = {
  render: () => (
    <SheetCard bare>
      <View className="items-center gap-3.5">
        <DeviceArt />
        <View className="items-center gap-1"><Title>Stephania-8F21</Title><Footnote>Stephania Connect</Footnote></View>
      </View>
      <PrimaryButton title="Set up" onPress={noop} />
    </SheetCard>
  ),
};

export const WifiPassword: Story = {
  render: () => (
    <SheetCard icon="wifi" title="Venue-Staff" subtitle="Stephania-8F21 joins this network">
      <Field label="Password for Venue-Staff" icon="lock" secure value="password" onChangeText={noop} />
      <PrimaryButton title="Join" onPress={noop} />
    </SheetCard>
  ),
};
