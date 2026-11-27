import type {Meta, StoryObj} from '@storybook/react-native';
import {useState} from 'react';
import {View} from 'react-native';
import {Field, Segmented, Toggle} from './controls';

const meta = {title: 'UI/Controls', component: Toggle, args: {value: true, label: 'HDMI audio on air', onChange: () => {}}} satisfies Meta<typeof Toggle>;
export default meta;
type Story = StoryObj<typeof meta>;

function ToggleDemo() {
  const [a, setA] = useState(true);
  const [b, setB] = useState(false);
  return (
    <View className="flex-row gap-4">
      <Toggle value={a} onChange={setA} label="On" />
      <Toggle value={b} onChange={setB} label="Off" />
      <Toggle value disabled onChange={() => {}} label="Disabled" />
    </View>
  );
}
export const Toggles: Story = {render: () => <ToggleDemo />};

function SegmentedDemo({locked}: {locked?: boolean}) {
  const [v, setV] = useState('rear');
  return (
    <Segmented label="Cameras" value={v} onChange={setV} locked={locked}>
      <Segmented.Item value="rear" label="Rear camera" icon="camera" />
      <Segmented.Item value="front" label="Front camera" icon="selfie" />
      <Segmented.Item value="dual" label="Both cameras" icon="bothCameras" unavailable />
    </Segmented>
  );
}
export const SegmentedIcons: Story = {render: () => <SegmentedDemo />};
export const SegmentedLocked: Story = {render: () => <SegmentedDemo locked />};

export const Fields: Story = {
  render: () => (
    <View className="gap-4">
      <Field label="Name shown on this phone" defaultValue="Hall A" />
      <Field label="Password for Venue-Staff" icon="lock" secure defaultValue="password" />
    </View>
  ),
};
