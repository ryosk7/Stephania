import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import {LiveBadge, Notice, StatusChip} from './status';

const meta = {
  title: 'UI/Status',
  component: StatusChip,
  args: {tone: 'live', icon: 'broadcast', label: 'Streaming', value: '12:04', pulse: true},
} satisfies Meta<typeof StatusChip>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Streaming: Story = {};
export const NotReachable: Story = {args: {tone: 'warning', icon: 'wifiOff', label: 'Not reachable', value: undefined, pulse: false}};

export const AllStates: Story = {
  render: () => (
    <View className="gap-3">
      <View className="flex-row flex-wrap items-center gap-2.5">
        <StatusChip tone="live" icon="broadcast" label="Streaming" value="12:04" pulse />
        <StatusChip tone="info" icon="refresh" label="Starting" />
        <StatusChip tone="neutral" icon="stop" label="Off air" />
        <StatusChip tone="warning" icon="wifiOff" label="Not reachable" />
        <StatusChip tone="success" icon="check" label="Done" />
        <LiveBadge elapsed="12:04" />
      </View>
      <Notice tone="warning" icon="thermometer">Warm · quality reduced</Notice>
      <Notice tone="neutral">Front camera paused to protect the phone.</Notice>
      <Notice tone="success">Hall A is ready</Notice>
    </View>
  ),
};
